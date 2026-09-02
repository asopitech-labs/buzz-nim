use nostr::{EventBuilder, EventId, Kind};
use sha2::{Digest, Sha256};

use super::{check_content, tag};

/// Kind 30620 — replaceable workflow definition.
///
/// The `d` tag carries the workflow id; `h` tag carries the channel id; the
/// content is the YAML definition. Same (pubkey, d) replaces the prior version.
pub fn build_workflow_definition(
    workflow_id: &str,
    channel_id: &str,
    yaml_definition: &str,
    expected_revision: Option<&str>,
) -> Result<EventBuilder, String> {
    check_content(yaml_definition)?;
    let mut tags = vec![tag(vec!["d", workflow_id])?, tag(vec!["h", channel_id])?];
    if let Some(revision) = expected_revision {
        EventId::from_hex(revision).map_err(|_| "invalid workflow revision".to_string())?;
        tags.push(tag(vec!["expected-revision", revision])?);
    }
    Ok(EventBuilder::new(Kind::Custom(30620), yaml_definition.to_string()).tags(tags))
}

/// Kind 5 — NIP-09 deletion targeting a kind:30620 workflow definition.
pub fn build_workflow_delete(
    workflow_id: &str,
    owner_pubkey_hex: &str,
) -> Result<EventBuilder, String> {
    let coord = format!("30620:{owner_pubkey_hex}:{workflow_id}");
    let tags = vec![tag(vec!["a", &coord])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

/// Kind 46020 — trigger a workflow run by id.
pub fn build_workflow_trigger(workflow_id: &str) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["d", workflow_id])?];
    Ok(EventBuilder::new(Kind::Custom(46020), "").tags(tags))
}

/// Kind 46030 — grant an approval token (with optional note).
pub fn build_approval_grant(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![approval_reference_tag(token)?];
    Ok(EventBuilder::new(Kind::Custom(46030), note.unwrap_or("")).tags(tags))
}

/// Kind 46031 — deny an approval token (with optional note).
pub fn build_approval_deny(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![approval_reference_tag(token)?];
    Ok(EventBuilder::new(Kind::Custom(46031), note.unwrap_or("")).tags(tags))
}

fn approval_reference_tag(token: &str) -> Result<nostr::Tag, String> {
    uuid::Uuid::parse_str(token).map_err(|_| "invalid approval token UUID".to_owned())?;
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    tag(vec!["d", &token_hash])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_commands_hash_the_raw_token_into_the_d_tag() {
        let token = "550e8400-e29b-41d4-a716-446655440000";
        let event = build_approval_grant(token, None)
            .expect("builder")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign");
        let expected = hex::encode(Sha256::digest(token.as_bytes()));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", expected.as_str()]));
        assert!(build_approval_deny("not-a-uuid", None).is_err());
    }
}
