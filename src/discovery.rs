use std::ffi::CStr;
use std::ffi::CString;
use std::future::Future;
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::os::raw::c_int;
use std::pin::Pin;
use std::time::Duration;

/// One observation from a Scan: a MAC address, the IPv4 address it had at
/// that moment, and an optional reverse-DNS hostname for that address.
#[derive(Clone, Debug)]
pub struct Observation {
    pub mac: String,
    pub ip: Ipv4Addr,
    pub hostname: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("scan failed: {0}")]
    Failed(String),
}

/// One Segment scan operation. Production sends raw ARP requests; tests
/// substitute a scripted fake ([`crate::discovery::fake::Scripted`]).
pub trait Discovery: Send + Sync {
    fn scan<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Observation>, DiscoveryError>> + Send + 'a>>;
}

/// Ethernet / ARP constants.
const ETH_P_ARP: u16 = 0x0806;
const ETH_P_IP: u16 = 0x0800;
const ARPHRD_ETHER: u16 = 1;
const ARPOP_REQUEST: u16 = 1;
const ARPOP_REPLY: u16 = 2;

/// How long the scan keeps listening for ARP replies after sending requests.
const SCAN_REPLY_WINDOW_SECS: i64 = 2;

/// How long to wait for a single PTR (reverse-DNS) answer before giving up.
/// A lookup that fails or times out yields no hostname and never fails the
/// scan: a network without working local DNS degrades to an empty marker.
const PTR_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The most targets we will send ARP requests to in one Scan. A home Segment
/// is a /24 (254 hosts); this bounds the worst case of a misconfigured wide
/// Segment so a startup Scan cannot hang sending packets forever.
const MAX_TARGETS: usize = 4096;

/// A single IPv4 Segment: a network address and its netmask.
#[derive(Clone, Debug)]
struct Segment {
    network: Ipv4Addr,
    mask: Ipv4Addr,
}

/// Production implementation: sweeps the Segment with raw ARP requests
/// (`AF_PACKET` / `SOCK_RAW` / `ETH_P_ARP`) and collects the replies.
pub struct ArpDiscovery {
    interface: String,
    segment: Segment,
    own_ip: Ipv4Addr,
    own_mac: [u8; 6],
}

impl ArpDiscovery {
    /// Build the production discovery for `interface`, deriving the Segment
    /// from the interface's primary IPv4 address and netmask, unless `subnet`
    /// gives an explicit CIDR override.
    pub fn new(interface: String, subnet: Option<String>) -> Result<Self, DiscoveryError> {
        let (own_ip, own_mac, network, mask) = derive_interface(&interface)?;
        let segment = match subnet {
            Some(cidr) => parse_cidr(&cidr)?,
            None => Segment { network, mask },
        };
        Ok(Self {
            interface,
            segment,
            own_ip,
            own_mac,
        })
    }
}

impl Discovery for ArpDiscovery {
    fn scan<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Observation>, DiscoveryError>> + Send + 'a>> {
        let interface = self.interface.clone();
        let segment = self.segment.clone();
        let own_ip = self.own_ip;
        let own_mac = self.own_mac;
        Box::pin(async move {
            let observations =
                tokio::task::spawn_blocking(move || sweep(&interface, &segment, own_ip, own_mac))
                    .await
                    .map_err(|error| {
                        DiscoveryError::Failed(format!("scan task ended unexpectedly: {error}"))
                    })??;
            // Reverse-DNS lookup per observed IP after the ARP sweep. Each
            // lookup runs on the blocking pool with a short timeout; a failure
            // yields no hostname and never fails the scan.
            let mut named = Vec::with_capacity(observations.len());
            let mut handles = Vec::with_capacity(observations.len());
            for obs in &observations {
                let ip = obs.ip;
                handles.push(tokio::task::spawn_blocking(move || ptr_lookup(ip)));
            }
            for (obs, handle) in observations.into_iter().zip(handles) {
                let hostname = tokio::time::timeout(PTR_LOOKUP_TIMEOUT, handle)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
                named.push(Observation {
                    mac: obs.mac,
                    ip: obs.ip,
                    hostname,
                });
            }
            Ok(named)
        })
    }
}

/// Derive the scanner's own IPv4 address, MAC address, and the network/netmask
/// of `interface` from the kernel interface table.
fn derive_interface(
    interface: &str,
) -> Result<(Ipv4Addr, [u8; 6], Ipv4Addr, Ipv4Addr), DiscoveryError> {
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return Err(DiscoveryError::Failed("getifaddrs failed".to_string()));
        }
        let mut own_ip = None;
        let mut mask = None;
        let mut entry = head;
        while !entry.is_null() {
            let item = &*entry;
            let name = CStr::from_ptr(item.ifa_name).to_string_lossy();
            if name == interface
                && !item.ifa_addr.is_null()
                && (*item.ifa_addr).sa_family as c_int == libc::AF_INET
            {
                let socket = &*(item.ifa_addr as *const libc::sockaddr as *const libc::sockaddr_in);
                own_ip = Some(Ipv4Addr::from(socket.sin_addr.s_addr.to_be_bytes()));
                if !item.ifa_netmask.is_null() {
                    let netmask =
                        &*(item.ifa_netmask as *const libc::sockaddr as *const libc::sockaddr_in);
                    mask = Some(Ipv4Addr::from(netmask.sin_addr.s_addr.to_be_bytes()));
                }
            }
            entry = item.ifa_next;
        }
        libc::freeifaddrs(head);

        let (own_ip, mask) = match (own_ip, mask) {
            (Some(ip), Some(mask)) => (ip, mask),
            _ => {
                return Err(DiscoveryError::Failed(format!(
                    "interface {interface} has no usable IPv4 address and netmask"
                )));
            }
        };

        let own_mac = interface_mac(interface)?;
        let network = ip_to_u32(own_ip) & ip_to_u32(mask);
        Ok((own_ip, own_mac, Ipv4Addr::from(network.to_be_bytes()), mask))
    }
}

/// Read the MAC address of `interface` via ioctl(SIOCGIFHWADDR).
fn interface_mac(interface: &str) -> Result<[u8; 6], DiscoveryError> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(DiscoveryError::Failed(
                "failed to create socket for interface query".to_string(),
            ));
        }
        let result = {
            let c_interface = CString::new(interface).map_err(|_| {
                DiscoveryError::Failed(format!("invalid interface name {interface:?}"))
            })?;
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name_bytes = c_interface.as_bytes_with_nul();
            for (i, &byte) in name_bytes[..name_bytes.len() - 1].iter().enumerate() {
                ifr.ifr_name[i] = byte as i8;
            }
            if libc::ioctl(fd, libc::SIOCGIFHWADDR, &mut ifr) != 0 {
                Err(DiscoveryError::Failed(format!(
                    "ioctl(SIOCGIFHWADDR) failed for {interface}"
                )))
            } else {
                Ok([
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[0] as u8,
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[1] as u8,
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[2] as u8,
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[3] as u8,
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[4] as u8,
                    ifr.ifr_ifru.ifru_hwaddr.sa_data[5] as u8,
                ])
            }
        };
        let _ = libc::close(fd);
        result
    }
}

fn parse_cidr(cidr: &str) -> Result<Segment, DiscoveryError> {
    let (address, prefix) = cidr
        .split_once('/')
        .ok_or_else(|| DiscoveryError::Failed(format!("invalid subnet {cidr:?}: missing '/'")))?;
    let ip: Ipv4Addr = address
        .parse()
        .map_err(|_| DiscoveryError::Failed(format!("invalid subnet {cidr:?}: bad address")))?;
    let prefix_len: u32 = prefix
        .parse()
        .map_err(|_| DiscoveryError::Failed(format!("invalid subnet {cidr:?}: bad prefix")))?;
    if prefix_len > 32 {
        return Err(DiscoveryError::Failed(format!(
            "invalid subnet {cidr:?}: prefix > 32"
        )));
    }
    let mask = if prefix_len == 0 {
        0u32
    } else {
        !((1u32 << (32 - prefix_len)) - 1)
    };
    let network = ip_to_u32(ip) & mask;
    Ok(Segment {
        network: Ipv4Addr::from(network.to_be_bytes()),
        mask: Ipv4Addr::from(mask.to_be_bytes()),
    })
}

/// Every usable host IPv4 address in the Segment, lowest first.
fn host_ips(segment: &Segment) -> Vec<Ipv4Addr> {
    let network = ip_to_u32(segment.network);
    let mask = ip_to_u32(segment.mask);
    let broadcast = network | !mask;
    let mut ips = Vec::new();
    let mut host = network + 1;
    while host < broadcast {
        ips.push(Ipv4Addr::from(host.to_be_bytes()));
        host += 1;
    }
    ips
}

/// One raw-ARP sweep of the Segment. Returns the (MAC, IPv4) pairs that
/// answered, excluding the scanner's own interface addresses.
fn sweep(
    interface: &str,
    segment: &Segment,
    own_ip: Ipv4Addr,
    own_mac: [u8; 6],
) -> Result<Vec<Observation>, DiscoveryError> {
    unsafe {
        let fd = libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW,
            libc::htons(ETH_P_ARP) as c_int,
        );
        if fd < 0 {
            return Err(DiscoveryError::Failed(
                "failed to create raw ARP socket (needs CAP_NET_RAW)".to_string(),
            ));
        }

        let result = {
            let c_interface = CString::new(interface).map_err(|_| {
                DiscoveryError::Failed(format!("invalid interface name {interface:?}"))
            })?;
            let ifindex = libc::if_nametoindex(c_interface.as_ptr());
            if ifindex == 0 {
                return Err(DiscoveryError::Failed(format!(
                    "interface {interface} not found"
                )));
            }

            let mut ll: libc::sockaddr_ll = std::mem::zeroed();
            ll.sll_family = libc::AF_PACKET as u16;
            ll.sll_ifindex = ifindex as c_int;
            ll.sll_protocol = libc::htons(ETH_P_ARP) as u16;
            if libc::bind(
                fd,
                &ll as *const libc::sockaddr_ll as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            ) != 0
            {
                return Err(DiscoveryError::Failed(format!(
                    "failed to bind raw socket to {interface}"
                )));
            }

            let timeout = libc::timeval {
                tv_sec: SCAN_REPLY_WINDOW_SECS,
                tv_usec: 0,
            };
            if libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &timeout as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            ) != 0
            {
                return Err(DiscoveryError::Failed(
                    "failed to set socket receive timeout".to_string(),
                ));
            }

            let targets = host_ips(segment);
            let targets = if targets.len() > MAX_TARGETS {
                eprintln!(
                    "lanlord: Segment has {} hosts, scanning only the first {MAX_TARGETS}",
                    targets.len()
                );
                targets.into_iter().take(MAX_TARGETS).collect::<Vec<_>>()
            } else {
                targets
            };

            let mut dest: libc::sockaddr_ll = std::mem::zeroed();
            dest.sll_family = libc::AF_PACKET as u16;
            dest.sll_ifindex = ifindex as c_int;
            dest.sll_protocol = libc::htons(ETH_P_ARP) as u16;
            dest.sll_halen = 6;
            dest.sll_addr[0..6].copy_from_slice(&[0xff; 6]);

            let mut frame = [0u8; 42];
            frame[0..6].copy_from_slice(&[0xff; 6]); // Ethernet dst = broadcast
            frame[6..12].copy_from_slice(&own_mac); // Ethernet src = own MAC
            frame[12..14].copy_from_slice(&ETH_P_ARP.to_be_bytes()); // ethertype = ARP
            frame[14..16].copy_from_slice(&ARPHRD_ETHER.to_be_bytes()); // hardware = Ethernet
            frame[16..18].copy_from_slice(&ETH_P_IP.to_be_bytes()); // protocol = IPv4
            frame[18] = 6; // hlen
            frame[19] = 4; // plen
            frame[20..22].copy_from_slice(&ARPOP_REQUEST.to_be_bytes()); // op = request
            frame[22..28].copy_from_slice(&own_mac); // sender hardware address
            frame[28..32].copy_from_slice(&own_ip.octets()); // sender protocol address
            frame[32..38].copy_from_slice(&[0u8; 6]); // target hardware address
            // target protocol address filled per target below

            for target in &targets {
                frame[38..42].copy_from_slice(&target.octets());
                let _ = libc::sendto(
                    fd,
                    frame.as_ptr() as *const libc::c_void,
                    frame.len(),
                    0,
                    &dest as *const libc::sockaddr_ll as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                );
            }

            let mut observations = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = libc::recvfrom(
                    fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if n < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut
                    {
                        break; // reply window closed
                    }
                    return Err(DiscoveryError::Failed(format!("recvfrom failed: {error}")));
                }
                let n = n as usize;
                if n < 42 {
                    continue; // Ethernet header + full ARP header
                }
                if u16::from_be_bytes([buf[12], buf[13]]) != ETH_P_ARP {
                    continue;
                }
                if u16::from_be_bytes([buf[20], buf[21]]) != ARPOP_REPLY {
                    continue;
                }
                let mac = [buf[22], buf[23], buf[24], buf[25], buf[26], buf[27]];
                let ip = Ipv4Addr::from([buf[28], buf[29], buf[30], buf[31]]);
                if mac == own_mac || ip == own_ip {
                    continue; // never report the scanner itself
                }
                let mac_text = format_mac(mac);
                if !observations
                    .iter()
                    .any(|obs: &Observation| obs.mac == mac_text && obs.ip == ip)
                {
                    observations.push(Observation {
                        mac: mac_text,
                        ip,
                        hostname: None,
                    });
                }
            }
            Ok(observations)
        };
        let _ = libc::close(fd);
        result
    }
}

fn format_mac(bytes: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

/// Reverse-DNS (PTR) lookup for `ip`: the hostname the system resolver knows
/// for that address, or `None` when there is no record or the resolver fails.
/// `NI_NAMEREQD` makes the resolver report failure instead of falling back to
/// the numeric address, so "unknown" stays unknown.
fn ptr_lookup(ip: Ipv4Addr) -> Option<String> {
    unsafe {
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as u16;
        // Copy the octets straight into `s_addr` memory: `ip.octets()` is
        // already in network byte order, so this is endianness neutral.
        std::ptr::copy_nonoverlapping(
            ip.octets().as_ptr(),
            &mut addr.sin_addr.s_addr as *mut u32 as *mut u8,
            4,
        );
        let mut host: [i8; 1024] = [0; 1024];
        let ret = libc::getnameinfo(
            &addr as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            host.as_mut_ptr(),
            host.len() as libc::socklen_t,
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        );
        if ret != 0 {
            return None;
        }
        let name = CStr::from_ptr(host.as_ptr()).to_string_lossy();
        (!name.is_empty()).then_some(name.into_owned())
    }
}

fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_be_bytes(ip.octets())
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Returns scripted observations, one per Scan call, in order.
    pub struct Scripted {
        outcomes: Mutex<Vec<Result<Vec<Observation>, DiscoveryError>>>,
    }

    impl Scripted {
        pub fn new(observations: Vec<Vec<Observation>>) -> Self {
            Self {
                outcomes: Mutex::new(observations.into_iter().map(Ok).collect()),
            }
        }
    }

    impl Discovery for Scripted {
        fn scan<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Observation>, DiscoveryError>> + Send + 'a>>
        {
            Box::pin(async move {
                let mut outcomes = self
                    .outcomes
                    .lock()
                    .expect("outcomes mutex is not poisoned");
                if outcomes.is_empty() {
                    return Err(DiscoveryError::Failed(
                        "scripted fake has no more scripted outcomes".to_string(),
                    ));
                }
                outcomes.remove(0)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scripted_fake_returns_observations_in_order() {
        let observation = Observation {
            mac: "aa:bb:cc:dd:ee:ff".to_string(),
            ip: "10.0.0.1".parse().unwrap(),
            hostname: None,
        };
        let fake = fake::Scripted::new(vec![vec![observation.clone()], vec![]]);
        let first = fake.scan().await.expect("first scan succeeds");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].mac, "aa:bb:cc:dd:ee:ff");
        let second = fake.scan().await.expect("second scan succeeds");
        assert!(second.is_empty());
        let third = fake.scan().await;
        assert!(
            third.is_err(),
            "scripted fake is exhausted after its outcomes"
        );
    }

    #[test]
    fn parse_cidr_derives_network_and_mask() {
        let segment = parse_cidr("10.0.0.0/24").expect("valid CIDR parses");
        assert_eq!(segment.network, "10.0.0.0".parse::<Ipv4Addr>().unwrap());
        assert_eq!(segment.mask, "255.255.255.0".parse::<Ipv4Addr>().unwrap());
    }

    #[test]
    fn host_ips_covers_usable_hosts_only() {
        let segment = Segment {
            network: "10.0.0.0".parse::<Ipv4Addr>().unwrap(),
            mask: "255.255.255.0".parse::<Ipv4Addr>().unwrap(),
        };
        let ips = host_ips(&segment);
        assert_eq!(ips.len(), 254);
        assert_eq!(ips.first(), Some(&"10.0.0.1".parse::<Ipv4Addr>().unwrap()));
        assert_eq!(ips.last(), Some(&"10.0.0.254".parse::<Ipv4Addr>().unwrap()));
    }
}
