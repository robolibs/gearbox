use super::WeatherSettings;
use bevy::prelude::*;

pub(super) struct Daylight {
    pub direction: Vec3,
    pub direct_color: Color,
    pub direct_strength: f32,
    pub fill_strength: f32,
    pub sun_radiance: Vec4,
    pub zenith: Vec4,
    pub horizon: Vec4,
    pub cloud_top: Vec4,
    pub cloud_bottom: Vec4,
    pub fog: Color,
    pub ambient: Color,
    pub indirect_top: Color,
    pub indirect_mid: Color,
    pub indirect_bottom: Color,
}

fn smooth(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn air_mass(elevation: f32) -> f32 {
    let elevation = elevation.max(0.0);
    1.0 / (elevation.to_radians().sin() + 0.50572 * (elevation + 6.07995).powf(-1.6364))
}

impl Daylight {
    pub fn from_settings(settings: &WeatherSettings) -> Self {
        let direction = settings
            .sun_position
            .try_normalize()
            .filter(|v| v.is_finite())
            .unwrap_or(Vec3::new(-4.0, 7.0, 5.0).normalize());
        let elevation = direction.y.clamp(-1.0, 1.0).asin().to_degrees();
        let reference = Vec3::new(-4.0, 7.0, 5.0).normalize().y.asin().to_degrees();
        let path = (air_mass(elevation) - air_mass(reference)).max(0.0);
        let transmission = Vec3::new(
            (-0.045 * path).exp(),
            (-0.09 * path).exp(),
            (-0.19 * path).exp(),
        );
        let base = settings.sun_color.to_linear().to_vec4().truncate();
        let rgb = base * transmission / transmission.max_element();
        let direct_strength =
            transmission.dot(Vec3::new(0.2126, 0.7152, 0.0722)) * smooth(-0.27, 0.27, elevation);
        let daylight = smooth(-6.0, 18.0, elevation);
        let warmth = 1.0 - smooth(0.0, 35.0, elevation);
        let twilight = smooth(-12.0, -1.0, elevation);
        let sky_strength = 0.005 + 0.995 * (0.16 * twilight + 0.84 * daylight);
        let blend = |day: Vec3, dusk: Vec3| day.lerp(dusk, warmth);
        let color = |v: Vec3| Color::linear_rgb(v.x, v.y, v.z);
        let zenith = blend(Vec3::new(0.20, 0.50, 0.85), Vec3::new(0.12, 0.17, 0.34)) * sky_strength;
        let horizon =
            blend(Vec3::new(0.70, 0.85, 0.95), Vec3::new(1.05, 0.38, 0.13)) * sky_strength;
        Self {
            direction,
            direct_color: color(rgb),
            direct_strength,
            fill_strength: 0.005 + 0.995 * smooth(-10.0, 4.0, elevation),
            sun_radiance: (rgb * (1.4 * direct_strength)).extend(1.0),
            zenith: zenith.extend(1.0),
            horizon: horizon.extend(1.0),
            cloud_top: (blend(
                settings.clouds.clouds_ambient_color_top.truncate(),
                Vec3::new(0.85, 0.40, 0.22),
            ) * sky_strength)
                .extend(0.0),
            cloud_bottom: (blend(
                settings.clouds.clouds_ambient_color_bottom.truncate(),
                Vec3::new(0.15, 0.12, 0.18),
            ) * sky_strength)
                .extend(0.0),
            fog: color(
                blend(
                    settings.fog_color.to_linear().to_vec4().truncate(),
                    Vec3::new(0.64, 0.29, 0.15),
                ) * sky_strength,
            ),
            ambient: color(blend(
                Color::srgb(0.80, 0.88, 1.0)
                    .to_linear()
                    .to_vec4()
                    .truncate(),
                Vec3::new(0.65, 0.63, 0.70),
            )),
            indirect_top: color(blend(
                Vec3::new(0.45, 0.65, 0.95),
                Vec3::new(0.40, 0.48, 0.70),
            )),
            indirect_mid: color(blend(
                Vec3::new(0.64, 0.70, 0.78),
                Vec3::new(0.70, 0.53, 0.40),
            )),
            indirect_bottom: color(blend(
                Vec3::new(0.20, 0.24, 0.15),
                Vec3::new(0.20, 0.16, 0.10),
            )),
        }
    }
}
