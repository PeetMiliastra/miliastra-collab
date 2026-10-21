//! Client-only reconciliation scheduling. No changes to coordinator messages or routes.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub const KEEPALIVE: Duration = Duration::from_secs(30);
pub const SOCKET_CHECK: Duration = Duration::from_secs(5);
pub const RECONNECT_DELAY: Duration = Duration::from_secs(10);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(90);
const CONNECTED_REFRESH: Duration = Duration::from_secs(10 * 60);
const FALLBACK_REFRESH: Duration = Duration::from_secs(30);
const RESERVATION_RECHECK_MARGIN_SECS: i64 = 30;
const EVENT_COALESCE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    endpoint: String,
    project: String,
    member: String,
}
impl Session {
    pub fn new(endpoint: &str, project: &str, member: &str) -> Self {
        Self {
            endpoint: endpoint.into(),
            project: project.into(),
            member: member.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refresh {
    None,
    Incremental,
    Full,
}
impl Refresh {
    pub fn for_event(kind: &str) -> Self {
        // A publication needs only the newly published pages. A reconcile notice means
        // the server dropped events, so retain the explicit full-history recovery path.
        if kind == "reconcile" {
            Self::Full
        } else {
            Self::Incremental
        }
    }
}
pub struct Work {
    pub session: Session,
    pub refresh: Refresh,
    pub grant_access: bool,
}
#[derive(Default)]
struct State {
    attempt: Option<(Session, Instant)>,
    socket: Option<(Session, Instant)>,
    pending: Option<Work>,
}
impl State {
    fn socket_alive(&self, session: &Session, now: Instant) -> bool {
        self.socket.as_ref().is_some_and(|(key, received)| {
            key == session && now.saturating_duration_since(*received) < SOCKET_TIMEOUT
        })
    }
    fn due(&self, session: &Session, online: bool, reservation_due: bool, now: Instant) -> bool {
        let interval = if online && !reservation_due && self.socket_alive(session, now) {
            CONNECTED_REFRESH
        } else {
            FALLBACK_REFRESH
        };
        self.attempt.as_ref().is_none_or(|(key, at)| {
            key != session || now.saturating_duration_since(*at) >= interval
        })
    }
    fn queue(&mut self, work: Work) {
        if let Some(pending) = self.pending.as_mut().filter(|p| p.session == work.session) {
            pending.refresh = pending.refresh.max(work.refresh);
            pending.grant_access |= work.grant_access;
        } else {
            self.pending = Some(work);
        }
    }
}
#[derive(Default)]
pub struct CoordinatorSync {
    state: Mutex<State>,
    requested: Notify,
}
impl CoordinatorSync {
    pub fn attempted(&self, session: Session, now: Instant) {
        self.state.lock().unwrap().attempt = Some((session, now));
    }
    pub fn received(&self, session: Session, now: Instant) {
        self.state.lock().unwrap().socket = Some((session, now));
    }
    pub fn disconnected(&self, session: &Session) {
        let mut state = self.state.lock().unwrap();
        if state.socket.as_ref().is_some_and(|(key, _)| key == session) {
            state.socket = None;
        }
    }
    pub fn socket_alive(&self, session: &Session, now: Instant) -> bool {
        self.state.lock().unwrap().socket_alive(session, now)
    }
    pub fn due(
        &self,
        session: &Session,
        online: bool,
        reservation_due: bool,
        now: Instant,
    ) -> bool {
        self.state
            .lock()
            .unwrap()
            .due(session, online, reservation_due, now)
    }
    pub fn request(&self, session: Session, refresh: Refresh, grant_access: bool) {
        self.state.lock().unwrap().queue(Work {
            session,
            refresh,
            grant_access,
        });
        self.requested.notify_one();
    }
    pub async fn next(&self) -> Work {
        loop {
            self.requested.notified().await;
            // A bounded window groups bursts without letting continuous activity
            // postpone reconciliation forever. The socket reader never waits on I/O here.
            tokio::time::sleep(EVENT_COALESCE).await;
            let work = self.state.lock().unwrap().pending.take();
            if let Some(work) = work {
                return work;
            }
        }
    }
}

pub fn reservation_due(expires: i64, now: i64) -> bool {
    expires <= now.saturating_add(RESERVATION_RECHECK_MARGIN_SECS)
}

pub fn history_needed(known: u64, sequence: u64, full: bool) -> bool {
    full || known < sequence
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session::new("https://service.example", "project", "member")
    }
    #[test]
    fn healthy_idle_connection_reconciles_144_times_per_day_instead_of_2880() {
        let sync = CoordinatorSync::default();
        let session = session();
        let start = Instant::now();
        sync.attempted(session.clone(), start);
        let mut requests = 0;
        for seconds in (2..=86400).step_by(2) {
            let now = start + Duration::from_secs(seconds);
            if seconds % KEEPALIVE.as_secs() == 0 {
                sync.received(session.clone(), now);
            }
            if sync.due(&session, true, false, now) {
                requests += 1;
                sync.attempted(session.clone(), now);
            }
        }
        assert_eq!(requests, 144);
    }
    #[test]
    fn dead_socket_or_failed_sync_restores_fast_fallback() {
        let sync = CoordinatorSync::default();
        let key = session();
        let start = Instant::now();
        sync.attempted(key.clone(), start);
        sync.received(key.clone(), start);
        assert!(!sync.due(&key, true, false, start + FALLBACK_REFRESH));
        assert!(sync.due(&key, false, false, start + FALLBACK_REFRESH));
        assert!(!sync.socket_alive(&key, start + SOCKET_TIMEOUT));
        assert!(sync.due(&key, true, false, start + SOCKET_TIMEOUT));
        sync.disconnected(&key);
        assert!(sync.due(&key, true, false, start + FALLBACK_REFRESH));
    }
    #[test]
    fn teammate_reservations_are_rechecked_before_the_cached_expiry() {
        let sync = CoordinatorSync::default();
        let key = session();
        let start = Instant::now();
        sync.attempted(key.clone(), start);
        sync.received(key.clone(), start + Duration::from_secs(80));
        assert!(!reservation_due(120, 89));
        assert!(reservation_due(120, 90));
        assert!(!sync.due(&key, true, false, start + Duration::from_secs(90)));
        assert!(sync.due(&key, true, true, start + Duration::from_secs(90)));
        sync.attempted(key.clone(), start + Duration::from_secs(90));
        // An expired cached record must not turn a failed HTTP request into a hot loop.
        assert!(!sync.due(&key, false, true, start + Duration::from_secs(92)));
        assert!(sync.due(&key, false, true, start + Duration::from_secs(120)));
    }
    #[test]
    fn another_project_account_or_server_never_inherits_socket_health() {
        let sync = CoordinatorSync::default();
        let key = session();
        let start = Instant::now();
        sync.attempted(key.clone(), start);
        sync.received(key.clone(), start);
        for other in [
            Session::new("https://other.example", "project", "member"),
            Session::new("https://service.example", "other", "member"),
            Session::new("https://service.example", "project", "other"),
        ] {
            assert!(!sync.socket_alive(&other, start));
            assert!(sync.due(&other, true, false, start));
            sync.disconnected(&other);
            assert!(sync.socket_alive(&key, start));
        }
    }
    #[tokio::test]
    async fn event_bursts_merge_and_requests_during_a_refresh_are_not_lost() {
        let sync = CoordinatorSync::default();
        let key = session();
        for _ in 0..100 {
            sync.request(key.clone(), Refresh::Incremental, false);
        }
        sync.request(key.clone(), Refresh::Full, false);
        sync.request(key.clone(), Refresh::None, true);
        let work = tokio::time::timeout(Duration::from_secs(2), sync.next())
            .await
            .unwrap();
        assert_eq!(work.refresh, Refresh::Full);
        assert!(work.grant_access);
        assert!(sync.state.lock().unwrap().pending.is_none());
        // A request arriving while the caller handles the previous batch is retained.
        sync.request(key, Refresh::Incremental, false);
        let next = tokio::time::timeout(Duration::from_secs(2), sync.next())
            .await
            .unwrap();
        assert_eq!(next.refresh, Refresh::Incremental);
        assert!(!next.grant_access);
    }
    #[tokio::test]
    async fn continuous_events_cannot_keep_postponing_reconciliation() {
        let sync = std::sync::Arc::new(CoordinatorSync::default());
        let producer = sync.clone();
        let task = tokio::spawn(async move {
            loop {
                producer.request(session(), Refresh::Incremental, false);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let work = tokio::time::timeout(Duration::from_secs(2), sync.next())
            .await
            .unwrap();
        task.abort();
        assert_eq!(work.refresh, Refresh::Incremental);
    }
    #[test]
    fn switching_projects_replaces_queued_work_without_leaking_full_refresh_or_access() {
        let sync = CoordinatorSync::default();
        sync.request(session(), Refresh::Full, true);
        let other = Session::new("https://service.example", "other", "member");
        sync.request(other.clone(), Refresh::Incremental, false);
        let work = sync.state.lock().unwrap().pending.take().unwrap();
        assert_eq!(work.session, other);
        assert_eq!(work.refresh, Refresh::Incremental);
        assert!(!work.grant_access);
    }
    #[test]
    fn cached_history_is_skipped_but_new_updates_and_explicit_recovery_are_fetched() {
        assert!(!history_needed(0, 0, false));
        assert!(!history_needed(100, 100, false));
        assert!(history_needed(0, 100, false));
        assert!(history_needed(100, 101, false));
        assert!(history_needed(100, 100, true));
        assert_eq!(Refresh::for_event("history"), Refresh::Incremental);
        assert_eq!(Refresh::for_event("reconcile"), Refresh::Full);
    }
}
