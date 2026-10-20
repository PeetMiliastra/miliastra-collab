//! Who has the app open. The coordinator only knows who is connected; the status itself
//! (for example "in Miliastra") is encrypted by the app.

/// Purpose string used when encrypting a member's status.
pub fn status_purpose(member: &str) -> String {
    format!("presence:{member}")
}
