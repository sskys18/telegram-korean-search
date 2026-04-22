pub fn calculate_trending_score(
    message_count: i64,
    last_seen_at: i64,
    msgs_24h: i64,
    msgs_7d: i64,
    unique_channels_7d: i64,
    total_active_channels: i64,
    now: i64,
) -> f64 {
    if message_count == 0 {
        return 0.0;
    }

    let hours_since_last = ((now - last_seen_at) as f64) / 3600.0;
    let recency = (-0.1 * hours_since_last).exp();

    let daily_avg_7d = (msgs_7d as f64) / 7.0;
    let velocity = (msgs_24h as f64) / daily_avg_7d.max(1.0);

    let total_channels = (total_active_channels as f64).max(1.0);
    let channel_div = (unique_channels_7d as f64) / total_channels;

    let base = (message_count as f64 + 1.0).log2();

    velocity * recency * base * channel_div
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trending_score_basic() {
        let now = 1700000000;
        let score = calculate_trending_score(100, now - 3600, 20, 50, 5, 20, now);
        assert!(score > 0.0);
    }

    #[test]
    fn test_trending_score_zero_messages() {
        let score = calculate_trending_score(0, 0, 0, 0, 0, 10, 1700000000);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_trending_score_recency_decay() {
        let now = 1700000000;
        let recent = calculate_trending_score(100, now - 3600, 10, 30, 5, 20, now);
        let old = calculate_trending_score(100, now - 86400 * 7, 10, 30, 5, 20, now);
        assert!(recent > old, "Recent topics should score higher");
    }

    #[test]
    fn test_trending_score_velocity_boost() {
        let now = 1700000000;
        let spiking = calculate_trending_score(100, now - 3600, 50, 50, 5, 20, now);
        let steady = calculate_trending_score(100, now - 3600, 7, 50, 5, 20, now);
        assert!(spiking > steady, "Spiking topics should score higher");
    }

    #[test]
    fn test_trending_score_channel_diversity() {
        let now = 1700000000;
        let diverse = calculate_trending_score(100, now - 3600, 10, 30, 15, 20, now);
        let narrow = calculate_trending_score(100, now - 3600, 10, 30, 1, 20, now);
        assert!(diverse > narrow, "More diverse topics should score higher");
    }
}
