use std::fmt;
use std::str::FromStr;

use crate::error::PowerError;

/// A CPU power profile, matching the exact string values the Linux kernel's
/// `energy_performance_preference` cpufreq sysfs attribute accepts. Both
/// `intel_pstate` (active/HWP mode) and `amd-pstate-epp` (kernel 6.x+)
/// expose this same file with these same five values, which is what makes
/// this enum a shared vocabulary across vendors rather than an Intel-first
/// concept AMD support gets bolted onto later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerProfile {
    Default,
    Performance,
    BalancePerformance,
    BalancePower,
    Power,
}

impl PowerProfile {
    pub const ALL: [PowerProfile; 5] = [
        PowerProfile::Default,
        PowerProfile::Performance,
        PowerProfile::BalancePerformance,
        PowerProfile::BalancePower,
        PowerProfile::Power,
    ];

    /// The exact string the kernel expects to be written to / reports back
    /// from `energy_performance_preference`.
    pub fn as_kernel_str(&self) -> &'static str {
        match self {
            PowerProfile::Default => "default",
            PowerProfile::Performance => "performance",
            PowerProfile::BalancePerformance => "balance_performance",
            PowerProfile::BalancePower => "balance_power",
            PowerProfile::Power => "power",
        }
    }
}

impl fmt::Display for PowerProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_kernel_str())
    }
}

impl FromStr for PowerProfile {
    type Err = PowerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "default" => Ok(PowerProfile::Default),
            "performance" => Ok(PowerProfile::Performance),
            "balance_performance" => Ok(PowerProfile::BalancePerformance),
            "balance_power" => Ok(PowerProfile::BalancePower),
            "power" => Ok(PowerProfile::Power),
            other => Err(PowerError::InvalidProfile(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_kernel_strings() {
        for profile in PowerProfile::ALL {
            let s = profile.as_kernel_str();
            assert_eq!(s.parse::<PowerProfile>().unwrap(), profile);
        }
    }

    #[test]
    fn rejects_unknown_values() {
        assert!("ultra".parse::<PowerProfile>().is_err());
        assert!("Performance".parse::<PowerProfile>().is_err()); // case-sensitive
    }
}
