/// Build a deep link to a specific message.
///
/// Public chats (with username): `https://t.me/{username}/{msg_id}`
/// Private chats (no username):  `tg://privatepost?channel={channel_id}&post={msg_id}`
pub fn build_link(chat_id: i64, username: Option<&str>, message_id: i64) -> String {
    match username {
        Some(uname) if !uname.is_empty() => {
            format!("https://t.me/{}/{}", uname, message_id)
        }
        _ => {
            // Private: channel_id = abs(chat_id) - 1_000_000_000_000
            let channel_id = chat_id.unsigned_abs().saturating_sub(1_000_000_000_000);
            format!(
                "tg://privatepost?channel={}&post={}",
                channel_id, message_id
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_link() {
        let link = build_link(-1001234567890, Some("mychannel"), 42);
        assert_eq!(link, "https://t.me/mychannel/42");
    }

    #[test]
    fn test_private_link() {
        let link = build_link(-1001234567890, None, 42);
        // channel_id = 1001234567890 - 1000000000000 = 1234567890
        assert_eq!(link, "tg://privatepost?channel=1234567890&post=42");
    }

    #[test]
    fn test_private_link_empty_username() {
        let link = build_link(-1001234567890, Some(""), 42);
        assert_eq!(link, "tg://privatepost?channel=1234567890&post=42");
    }

    #[test]
    fn test_public_link_with_large_id() {
        let link = build_link(-1009999999999, Some("bigchat"), 999);
        assert_eq!(link, "https://t.me/bigchat/999");
    }

    #[test]
    fn test_private_link_positive_id() {
        // Edge case: positive chat_id (shouldn't happen for channels but handle gracefully)
        let link = build_link(12345, None, 1);
        assert_eq!(link, "tg://privatepost?channel=0&post=1");
    }
}
