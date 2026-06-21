use wgpu::PowerPreference;

#[derive(Clone, Debug)]
pub struct RenderProfile {
    pub name: &'static str,
    pub window_title: &'static str,
    pub target_width: u32,
    pub target_height: u32,
    pub instance_count: usize,
    pub layer_count: usize,
    pub camera_radius: f32,
    pub camera_speed: f32,
    pub power_preference: PowerPreference,
}

impl RenderProfile {
    pub fn pi5() -> Self {
        Self {
            name: "pi5",
            window_title: "chipviz modern - Raspberry Pi 5",
            target_width: 3840,
            target_height: 2160,
            instance_count: 4096,
            layer_count: 24,
            camera_radius: 7.5,
            camera_speed: 0.18,
            power_preference: PowerPreference::LowPower,
        }
    }

    pub fn m1() -> Self {
        Self {
            name: "m1",
            window_title: "chipviz modern - Apple Silicon",
            target_width: 3840,
            target_height: 2160,
            instance_count: 16384,
            layer_count: 64,
            camera_radius: 9.5,
            camera_speed: 0.28,
            power_preference: PowerPreference::HighPerformance,
        }
    }
}
