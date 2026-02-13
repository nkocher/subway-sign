use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::models::Alert;

/// GTFS-RT effect enum → priority mapping.
/// Lower number = higher priority (more critical).
pub(crate) fn effect_priority(effect_value: i32) -> i32 {
    match effect_value {
        1 => 1,  // NO_SERVICE
        2 => 2,  // REDUCED_SERVICE
        3 => 3,  // SIGNIFICANT_DELAYS
        4 => 4,  // DETOUR
        5 => 5,  // ADDITIONAL_SERVICE
        6 => 6,  // MODIFIED_SERVICE
        7 => 7,  // OTHER_EFFECT
        8 => 8,  // UNKNOWN_EFFECT
        9 => 9,  // STOP_MOVED
        _ => 10, // Unknown
    }
}

/// Cooldown period — don't show same alert for this long.
const COOLDOWN_SECONDS: u64 = 300; // 5 minutes

/// Maximum alerts to queue.
const MAX_QUEUE_SIZE: usize = 10;

/// Manages alert filtering, prioritization, and cooldown tracking.
pub struct AlertManager {
    /// Cooldown tracking: alert_key → last displayed instant.
    cooldowns: HashMap<String, Instant>,
    /// Current alert queue (filtered, sorted, ready to display).
    queue: Vec<Alert>,
    /// Current position in queue.
    queue_index: usize,
    /// Track which alerts have been shown this cycle.
    shown_this_cycle: HashSet<String>,
    /// Last cleanup instant.
    last_cleanup: Instant,
}

impl AlertManager {
    pub fn new() -> Self {
        AlertManager {
            cooldowns: HashMap::new(),
            queue: Vec::new(),
            queue_index: 0,
            shown_this_cycle: HashSet::new(),
            last_cleanup: Instant::now(),
        }
    }

    /// Filter alerts by priority and apply cooldown.
    pub fn filter_and_sort(&mut self, alerts: &[Alert]) -> Vec<Alert> {
        self.cleanup_cooldowns();

        // Filter by cooldown
        let mut non_cooled: Vec<Alert> = alerts
            .iter()
            .filter(|a| !self.is_on_cooldown(a))
            .cloned()
            .collect();

        // Sort by priority (lower = more important)
        non_cooled.sort_by_key(|a| a.priority);

        // Cap queue size
        non_cooled.truncate(MAX_QUEUE_SIZE);

        // Update queue
        self.queue = non_cooled.clone();
        if self.queue_index >= self.queue.len() {
            self.queue_index = 0;
        }

        non_cooled
    }

    /// Get the next alert to display from the queue.
    pub fn get_next_alert(&self) -> Option<&Alert> {
        if self.queue.is_empty() {
            return None;
        }

        let mut checked = 0;
        let mut idx = self.queue_index;
        while checked < self.queue.len() {
            let alert = &self.queue[idx];
            let key = Self::alert_key(alert);

            if !self.shown_this_cycle.contains(&key) && !self.is_on_cooldown(alert) {
                return Some(alert);
            }

            idx = (idx + 1) % self.queue.len();
            checked += 1;
        }

        None
    }

    /// Advance to the next alert in the queue.
    fn advance_queue(&mut self) {
        if !self.queue.is_empty() {
            self.queue_index = (self.queue_index + 1) % self.queue.len();
        }
    }

    /// Mark an alert as displayed, starting its cooldown.
    pub fn mark_displayed(&mut self, alert: &Alert) {
        let key = Self::alert_key(alert);
        self.cooldowns.insert(key.clone(), Instant::now());
        self.shown_this_cycle.insert(key);
        self.advance_queue();
    }

    /// Reset the cycle tracking.
    pub fn reset_cycle(&mut self) {
        self.shown_this_cycle.clear();
        self.queue_index = 0;
    }

    /// Check if all alerts in the queue have been shown this cycle.
    pub fn all_shown_this_cycle(&self) -> bool {
        if self.queue.is_empty() {
            return true;
        }
        self.queue
            .iter()
            .all(|a| self.shown_this_cycle.contains(&Self::alert_key(a)))
    }

    /// Number of alerts currently in queue.
    #[cfg(test)]
    pub(crate) fn queue_size(&self) -> usize {
        self.queue.len()
    }

    /// Check if there are any displayable alerts (not on cooldown).
    pub fn has_alerts(&self) -> bool {
        self.queue.iter().any(|a| !self.is_on_cooldown(a))
    }

    /// Run periodic cleanup if enough time has passed.
    pub fn periodic_cleanup(&mut self) {
        if self.last_cleanup.elapsed().as_secs() > 60 {
            self.cleanup_cooldowns();
        }
    }

    fn alert_key(alert: &Alert) -> String {
        if !alert.alert_id.is_empty() {
            alert.alert_id.clone()
        } else {
            alert.text.chars().take(100).collect()
        }
    }

    fn is_on_cooldown(&self, alert: &Alert) -> bool {
        let key = Self::alert_key(alert);
        match self.cooldowns.get(&key) {
            Some(last_shown) => last_shown.elapsed().as_secs() < COOLDOWN_SECONDS,
            None => false,
        }
    }

    fn cleanup_cooldowns(&mut self) {
        let cutoff = COOLDOWN_SECONDS * 2;
        self.cooldowns
            .retain(|_, instant| instant.elapsed().as_secs() < cutoff);
        self.last_cleanup = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_alert(id: &str, text: &str, priority: i32) -> Alert {
        Alert {
            text: text.to_string(),
            affected_routes: HashSet::from(["1".to_string()]),
            priority,
            alert_id: id.to_string(),
        }
    }

    #[test]
    fn test_effect_priority() {
        assert_eq!(effect_priority(1), 1); // NO_SERVICE
        assert_eq!(effect_priority(3), 3); // SIGNIFICANT_DELAYS
        assert_eq!(effect_priority(99), 10); // Unknown
    }

    #[test]
    fn test_filter_and_sort() {
        let mut mgr = AlertManager::new();
        let alerts = vec![
            make_alert("a1", "Low priority", 5),
            make_alert("a2", "High priority", 1),
            make_alert("a3", "Medium priority", 3),
        ];

        let filtered = mgr.filter_and_sort(&alerts);
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].priority, 1); // sorted by priority
        assert_eq!(filtered[1].priority, 3);
        assert_eq!(filtered[2].priority, 5);
    }

    #[test]
    fn test_get_next_alert() {
        let mut mgr = AlertManager::new();
        let alerts = vec![
            make_alert("a1", "First", 1),
            make_alert("a2", "Second", 2),
        ];

        mgr.filter_and_sort(&alerts);

        let next = mgr.get_next_alert().unwrap();
        assert_eq!(next.alert_id, "a1");
    }

    #[test]
    fn test_mark_displayed_and_advance() {
        let mut mgr = AlertManager::new();
        let alerts = vec![
            make_alert("a1", "First", 1),
            make_alert("a2", "Second", 2),
        ];
        mgr.filter_and_sort(&alerts);

        // Show first alert
        let alert = mgr.get_next_alert().unwrap().clone();
        mgr.mark_displayed(&alert);

        // Next should be second alert
        let next = mgr.get_next_alert().unwrap();
        assert_eq!(next.alert_id, "a2");
    }

    #[test]
    fn test_all_shown_this_cycle() {
        let mut mgr = AlertManager::new();
        let alerts = vec![
            make_alert("a1", "First", 1),
            make_alert("a2", "Second", 2),
        ];
        mgr.filter_and_sort(&alerts);
        assert!(!mgr.all_shown_this_cycle());

        let a1 = mgr.get_next_alert().unwrap().clone();
        mgr.mark_displayed(&a1);
        assert!(!mgr.all_shown_this_cycle());

        let a2 = mgr.get_next_alert().unwrap().clone();
        mgr.mark_displayed(&a2);
        assert!(mgr.all_shown_this_cycle());
    }

    #[test]
    fn test_reset_cycle() {
        let mut mgr = AlertManager::new();
        let alerts = vec![make_alert("a1", "First", 1)];
        mgr.filter_and_sort(&alerts);

        let a1 = mgr.get_next_alert().unwrap().clone();
        mgr.mark_displayed(&a1);
        assert!(mgr.all_shown_this_cycle());

        mgr.reset_cycle();
        assert!(!mgr.all_shown_this_cycle());
    }

    #[test]
    fn test_empty_queue() {
        let mgr = AlertManager::new();
        assert!(mgr.get_next_alert().is_none());
        assert!(mgr.all_shown_this_cycle());
        assert_eq!(mgr.queue_size(), 0);
        assert!(!mgr.has_alerts());
    }

    #[test]
    fn test_queue_size_cap() {
        let mut mgr = AlertManager::new();
        let alerts: Vec<Alert> = (0..20)
            .map(|i| make_alert(&format!("a{}", i), &format!("Alert {}", i), i))
            .collect();
        mgr.filter_and_sort(&alerts);
        assert_eq!(mgr.queue_size(), MAX_QUEUE_SIZE);
    }
}
