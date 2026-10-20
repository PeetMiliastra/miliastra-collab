//! Renews reservations, independently of file transfers and stage changes.
use futures_util::{stream, StreamExt};
use miliastra_core::{constants::*, Lease};
use std::{collections::BTreeMap, future::Future};
pub async fn renew<F, Fut>(
    leases: BTreeMap<String, Lease>,
    request: F,
) -> Vec<(String, Lease, anyhow::Result<Lease>)>
where
    F: Fn(String, Lease) -> Fut,
    Fut: Future<Output = anyhow::Result<Lease>>,
{
    stream::iter(leases)
        .map(|(id, lease)| {
            let pending = request(id.clone(), lease.clone());
            async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(LEASE_RENEW_TIMEOUT_SECS),
                    pending,
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Reservation renewal timed out")));
                (id, lease, result)
            }
        })
        .buffer_unordered(LEASE_RENEW_CONCURRENCY)
        .collect()
        .await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn renewals_overlap_and_one_failure_does_not_starve_others() {
        let leases = (0..32)
            .map(|i| {
                (
                    i.to_string(),
                    Lease {
                        section: i.to_string(),
                        member: "me".into(),
                        name: "Me".into(),
                        fence: 1,
                        expires: 120,
                    },
                )
            })
            .collect();
        let active = std::sync::atomic::AtomicUsize::new(0);
        let peak = std::sync::atomic::AtomicUsize::new(0);
        let results = renew(leases, |id, lease| {
            let active = &active;
            let peak = &peak;
            async move {
                let count = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(count, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if id == "0" {
                    anyhow::bail!("transport failed")
                }
                Ok(lease)
            }
        })
        .await;
        assert_eq!(results.len(), 32);
        assert_eq!(results.iter().filter(|r| r.2.is_ok()).count(), 31);
        let peak = peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(peak > 1 && peak <= LEASE_RENEW_CONCURRENCY);
    }
}
