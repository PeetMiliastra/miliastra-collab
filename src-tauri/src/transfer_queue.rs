//! Runs transfers one at a time and remembers failed downloads for retrying.
use crate::db::Db;
use anyhow::Result;
#[derive(Default)]
pub struct TransferQueue {
    pub downloads: tokio::sync::Mutex<()>,
    pub publication: tokio::sync::Mutex<()>,
}
const RETRY_BASE_SECS: i64 = 5;
const MAX_BACKOFF_EXPONENT: u32 = 6;
fn retry_delay(attempt: u32) -> i64 {
    RETRY_BASE_SECS << attempt.min(MAX_BACKOFF_EXPONENT)
}
impl TransferQueue {
    pub fn ready(&self, db: &Db, hash: &str, now: i64) -> Result<bool> {
        Ok(db
            .get::<i64>(&format!("download-retry:{hash}"))?
            .unwrap_or(0)
            <= now)
    }
    pub fn completed(&self, db: &Db, hash: &str) -> Result<()> {
        db.set(&format!("download-state:{hash}"), &"downloaded")?;
        for key in ["error", "issue", "attempts", "retry"] {
            db.remove(&format!("download-{key}:{hash}"))?;
        }
        Ok(())
    }
    /// True only on the first failure, so the creator is told once.
    pub fn failed(&self, db: &Db, hash: &str, error: &anyhow::Error, now: i64) -> Result<bool> {
        let attempts = db
            .get::<u32>(&format!("download-attempts:{hash}"))?
            .unwrap_or(0)
            .saturating_add(1);
        db.set(&format!("download-attempts:{hash}"), &attempts)?;
        db.set(
            &format!("download-issue:{hash}"),
            &crate::recovery::classify(error),
        )?;
        db.set(&format!("download-error:{hash}"), &error.to_string())?;
        db.set(
            &format!("download-retry:{hash}"),
            &(now + retry_delay(attempts)),
        )?;
        Ok(attempts == 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_bounded() {
        assert_eq!(retry_delay(1), 10);
        assert_eq!(retry_delay(6), 320);
        assert_eq!(retry_delay(u32::MAX), 320);
    }
    #[test]
    fn retry_state_survives_restart_and_clears_after_success() {
        let root = std::env::temp_dir().join(format!("collab-transfer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let db = Db::open(&root.join("state.db")).unwrap();
            let queue = TransferQueue::default();
            assert!(queue
                .failed(&db, "hash", &anyhow::anyhow!("offline"), 100)
                .unwrap());
            assert!(!queue.ready(&db, "hash", 109).unwrap());
        }
        {
            let db = Db::open(&root.join("state.db")).unwrap();
            let queue = TransferQueue::default();
            assert!(queue.ready(&db, "hash", 110).unwrap());
            assert!(!queue
                .failed(&db, "hash", &anyhow::anyhow!("offline"), 110)
                .unwrap());
            assert!(!queue.ready(&db, "hash", 129).unwrap());
            queue.completed(&db, "hash").unwrap();
            assert!(queue.ready(&db, "hash", 129).unwrap());
            assert!(db.get::<String>("download-error:hash").unwrap().is_none());
            assert!(queue
                .failed(&db, "hash", &anyhow::anyhow!("offline"), 130)
                .unwrap());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
