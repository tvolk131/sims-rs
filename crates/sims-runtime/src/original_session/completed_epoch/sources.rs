//! Source-owned metadata audit after independently linking the retained Program.
pub fn evaluate(
    session: &super::super::Session,
    qualification: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    super::super::managed::completed_owned::audit::evaluate(session, qualification)
}
