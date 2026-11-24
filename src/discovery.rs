use std::future::Future;
use std::pin::Pin;

/// One observation from a Scan: a MAC address and the IPv4 address it had at
/// that moment.
#[derive(Clone)]
pub struct Observation {
    pub mac: String,
    pub ip: std::net::Ipv4Addr,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("ARP scanning is not implemented yet (see issue #3)")]
    Unimplemented,
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

/// Production implementation: stubbed until issue #3 wires up raw ARP.
pub struct ArpDiscovery;

impl Discovery for ArpDiscovery {
    fn scan<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Observation>, DiscoveryError>> + Send + 'a>> {
        Box::pin(async { Err(DiscoveryError::Unimplemented) })
    }
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
}
