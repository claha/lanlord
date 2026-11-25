use std::ffi::CStr;
use std::ffi::CString;
use std::net::Ipv4Addr;

/// Read the MAC address of `interface` via ioctl(SIOCGIFHWADDR).
///
/// Returns an error when the socket cannot be created, the interface name is
/// invalid, or the ioctl fails. Callers that cannot determine the scanner's own
/// MAC should degrade gracefully (e.g. by not excluding any MAC) rather than
/// failing the whole scan.
pub fn mac_of(interface: &str) -> Result<[u8; 6], std::io::Error> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = (|| {
            let c_interface = CString::new(interface).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid interface name {interface:?}"),
                )
            })?;
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name_bytes = c_interface.as_bytes_with_nul();
            for (i, &byte) in name_bytes[..name_bytes.len() - 1].iter().enumerate() {
                ifr.ifr_name[i] = byte as i8;
            }
            if libc::ioctl(fd, libc::SIOCGIFHWADDR, &mut ifr) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok([
                ifr.ifr_ifru.ifru_hwaddr.sa_data[0] as u8,
                ifr.ifr_ifru.ifru_hwaddr.sa_data[1] as u8,
                ifr.ifr_ifru.ifru_hwaddr.sa_data[2] as u8,
                ifr.ifr_ifru.ifru_hwaddr.sa_data[3] as u8,
                ifr.ifr_ifru.ifru_hwaddr.sa_data[4] as u8,
                ifr.ifr_ifru.ifru_hwaddr.sa_data[5] as u8,
            ])
        })();
        let _ = libc::close(fd);
        result
    }
}

/// Render a 6-byte MAC address as `"aa:bb:cc:dd:ee:ff"`.
pub fn format_mac(bytes: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

/// The IPv4 addresses assigned to `interface`, in kernel order.
pub fn ipv4_addrs_of(interface: &str) -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return ips;
        }
        let mut entry = head;
        while !entry.is_null() {
            let item = &*entry;
            let name = CStr::from_ptr(item.ifa_name).to_string_lossy();
            if name == interface
                && !item.ifa_addr.is_null()
                && (*item.ifa_addr).sa_family as std::os::raw::c_int == libc::AF_INET
            {
                let socket = &*(item.ifa_addr as *const libc::sockaddr as *const libc::sockaddr_in);
                ips.push(Ipv4Addr::from(socket.sin_addr.s_addr.to_be_bytes()));
            }
            entry = item.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    ips
}

/// The primary IPv4 address and netmask of `interface`, when it has one.
pub fn interface_ipv4_with_netmask(interface: &str) -> Option<(Ipv4Addr, Ipv4Addr)> {
    let mut ip = None;
    let mut mask = None;
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return None;
        }
        let mut entry = head;
        while !entry.is_null() {
            let item = &*entry;
            let name = CStr::from_ptr(item.ifa_name).to_string_lossy();
            if name == interface
                && !item.ifa_addr.is_null()
                && (*item.ifa_addr).sa_family as std::os::raw::c_int == libc::AF_INET
            {
                let socket = &*(item.ifa_addr as *const libc::sockaddr as *const libc::sockaddr_in);
                ip = Some(Ipv4Addr::from(socket.sin_addr.s_addr.to_be_bytes()));
                if !item.ifa_netmask.is_null() {
                    let netmask =
                        &*(item.ifa_netmask as *const libc::sockaddr as *const libc::sockaddr_in);
                    mask = Some(Ipv4Addr::from(netmask.sin_addr.s_addr.to_be_bytes()));
                }
            }
            entry = item.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    match (ip, mask) {
        (Some(ip), Some(mask)) => Some((ip, mask)),
        _ => None,
    }
}
