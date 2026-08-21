#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum CorePreset {
    Four,    // 4 physical cores / 8 threads (SMT on)
    Eight,   // 8 physical cores / 16 threads (SMT on)
    Twelve,  // 6 cores per CCD / 24 threads (SMT on)
    #[default]
    Sixteen, // all 16 physical cores / 32 threads
}

impl CorePreset {
    pub fn as_u32(&self) -> u32 {
        match self { Self::Four => 4, Self::Eight => 8, Self::Twelve => 12, Self::Sixteen => 16 }
    }

    pub fn from_u32(n: u32) -> Self {
        match n { 4 => Self::Four, 8 => Self::Eight, 12 => Self::Twelve, _ => Self::Sixteen }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Four    => "4C (8T)",
            Self::Eight   => "8C (16T)",
            Self::Twelve  => "12C / 6 per CCD (24T)",
            Self::Sixteen => "16C (32T)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Profile {
    Quiet,
    Balanced,
    Performance,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::Quiet => "quiet",
            Profile::Balanced => "balanced",
            Profile::Performance => "performance",
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Profile::Balanced
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PptLimits {
    pub apu_limit: u32,  // mW — sustain (stapm)
    pub fast_limit: u32, // mW — short-term burst
    pub slow_limit: u32, // mW — long-term burst
}

impl PptLimits {
    pub fn is_valid(&self) -> bool {
        self.slow_limit <= self.fast_limit
    }
}

impl Default for PptLimits {
    fn default() -> Self {
        PptLimits {
            apu_limit: 15000,
            fast_limit: 45000,
            slow_limit: 35000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FanCurve {
    pub points: Vec<(f32, f32)>, // (temp_°C, fan_speed_%)
    pub hysteresis: u8,          // °C buffer
}

impl FanCurve {
    pub fn shift(&mut self, delta: i32) {
        for (temp, _) in &mut self.points {
            *temp = (*temp + delta as f32).clamp(0.0, 100.0);
        }
    }
}

impl Default for FanCurve {
    fn default() -> Self {
        FanCurve {
            points: vec![
                (30.0,  0.0),
                (40.0, 10.0),
                (50.0, 20.0),
                (60.0, 40.0),
                (70.0, 60.0),
                (80.0, 80.0),
                (90.0, 90.0),
                (100.0, 100.0),
            ],
            hysteresis: 2,
        }
    }
}

pub struct AppState {
    pub profile: Profile,
    pub ppt: PptLimits,
    pub fan_curve: FanCurve,
    pub boost_enabled: bool,
    pub smt_enabled: bool,
    pub core_preset: CorePreset,
    /// Max CPU frequency cap in kHz. `None` means uncapped — the hardware
    /// maximum applies.
    pub max_freq_khz: Option<u32>,
    /// Hardware frequency range (min, max) in kHz as reported by cpufreq.
    /// `None` where the platform cannot report it (Windows), which also means
    /// the UI has no bounds to validate against.
    pub freq_range_khz: Option<(u32, u32)>,
    /// Highest live core frequency in MHz — the best single-number proxy for
    /// "what is the CPU running at", since cores clock independently.
    pub current_cpu_freq_mhz: Option<u32>,
    /// True when a core-count change has been applied but needs a reboot to take
    /// effect (Windows `bcdedit numproc`). Always false on Linux.
    pub core_reboot_pending: bool,
    pub current_temp: f32,
    pub current_gpu_temp: Option<f32>,
    pub current_cpu_fan_rpm: Option<u32>,
    pub current_gpu_fan_rpm: Option<u32>,
    pub current_battery_discharge_w: Option<f32>,
    /// Minutes of battery left at the current draw; `None` on AC.
    pub current_battery_minutes_left: Option<u32>,
    pub current_power: f32,
    pub status_msg: String,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            profile: Profile::default(),
            ppt: PptLimits::default(),
            fan_curve: FanCurve::default(),
            boost_enabled: true,
            smt_enabled: true,
            core_preset: CorePreset::default(),
            max_freq_khz: None,
            freq_range_khz: None,
            current_cpu_freq_mhz: None,
            core_reboot_pending: false,
            current_temp: 0.0,
            current_gpu_temp: None,
            current_cpu_fan_rpm: None,
            current_gpu_fan_rpm: None,
            current_battery_discharge_w: None,
            current_battery_minutes_left: None,
            current_power: 0.0,
            status_msg: String::from("Ready"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppt_valid_when_slow_lte_fast() {
        let ppt = PptLimits { apu_limit: 65000, slow_limit: 55000, fast_limit: 65000 };
        assert!(ppt.is_valid());
    }

    #[test]
    fn ppt_valid_when_apu_exceeds_slow() {
        // STAPM is independent of PPT slow — this is a valid real-world config
        let ppt = PptLimits { apu_limit: 65000, slow_limit: 55000, fast_limit: 65000 };
        assert!(ppt.is_valid());
    }

    #[test]
    fn ppt_invalid_when_slow_exceeds_fast() {
        let ppt = PptLimits { apu_limit: 15000, slow_limit: 50000, fast_limit: 45000 };
        assert!(!ppt.is_valid());
    }

    #[test]
    fn fan_curve_shift_clamps_to_range() {
        let mut curve = FanCurve::default();
        curve.shift(-100);
        for (temp, _) in &curve.points {
            assert_eq!(*temp, 0.0);
        }
        curve.shift(200);
        for (temp, _) in &curve.points {
            assert_eq!(*temp, 100.0);
        }
    }

    #[test]
    fn fan_curve_shift_adds_delta() {
        let mut curve = FanCurve {
            points: vec![(40.0, 20.0), (60.0, 50.0)],
            hysteresis: 2,
        };
        curve.shift(5);
        assert_eq!(curve.points[0].0, 45.0);
        assert_eq!(curve.points[1].0, 65.0);
    }
}
