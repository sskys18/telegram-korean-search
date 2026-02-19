/// Build a deep link to a specific message.
///
/// DMs (with username):          `https://t.me/{username}`
/// DMs (no username):            `tg://user?id={chat_id}`
/// Public chats (with username): `https://t.me/{username}/{msg_id}`
/// Private chats (no username):  `tg://privatepost?channel={channel_id}&post={msg_id}`
pub fn build_link(
    chat_id: i64,
    username: Option<&str>,
    message_id: i64,
    chat_type: &str,
) -> String {
    if chat_type == "dm" {
        return match username {
            Some(uname) if !uname.is_empty() => format!("https://t.me/{}", uname),
            _ => format!("tg://user?id={}", chat_id),
        };
    }

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
        let link = build_link(-1001234567890, Some("mychannel"), 42, "supergroup");
        assert_eq!(link, "https://t.me/mychannel/42");
    }

    #[test]
    fn test_private_link() {
        let link = build_link(-1001234567890, None, 42, "supergroup");
        // channel_id = 1001234567890 - 1000000000000 = 1234567890
        assert_eq!(link, "tg://privatepost?channel=1234567890&post=42");
    }

    #[test]
    fn test_private_link_empty_username() {
        let link = build_link(-1001234567890, Some(""), 42, "supergroup");
        assert_eq!(link, "tg://privatepost?channel=1234567890&post=42");
    }

    #[test]
    fn test_public_link_with_large_id() {
        let link = build_link(-1009999999999, Some("bigchat"), 999, "supergroup");
        assert_eq!(link, "https://t.me/bigchat/999");
    }

    #[test]
    fn test_private_link_positive_id() {
        // Edge case: positive chat_id (shouldn't happen for channels but handle gracefully)
        let link = build_link(12345, None, 1, "group");
        assert_eq!(link, "tg://privatepost?channel=0&post=1");
    }

    #[test]
    fn test_dm_link_with_username() {
        let link = build_link(12345, Some("johndoe"), 42, "dm");
        assert_eq!(link, "https://t.me/johndoe");
    }

    #[test]
    fn test_dm_link() {
        let link = build_link(12345, None, 42, "dm");
        assert_eq!(link, "tg://user?id=12345");
    }
}
