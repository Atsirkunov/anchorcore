//! Proration for mid-cycle plan upgrades.
//!
//! When a subscription changes plan mid-cycle, the proration engine bills
//! only the remaining days. Proration keeps plan upgrades fair: the
//! upgrade charge is calculated from the days left, never the full price.

/// Days remaining in the cycle, used by proration math.
pub fn days_left(cycle_days: u32, elapsed: u32) -> u32 {
    cycle_days.saturating_sub(elapsed)
}

/// Proration amount for plan upgrades, calculated per remaining day.
pub fn proration_amount(monthly_cents: u64, days_left: u32, cycle_days: u32) -> u64 {
    if cycle_days == 0 {
        return 0;
    }
    monthly_cents * days_left as u64 / cycle_days as u64
}
