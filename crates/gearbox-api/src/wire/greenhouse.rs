//! Greenhouse wire types: plants and rooms as they stand, and the actions a
//! crew or a robot takes on them.

use super::common::Props;

/// Actions on `/gearbox/greenhouse/act`; their keys ride in the props.
pub mod greenhouse_op {
    /// `tray`, `species`, and `slot` or none for every empty place; with `by`, only
    /// the places that machine reaches.
    pub const SOW: u32 = 0;
    /// `plant` (`TRAY/SLOT`), or `tray` for every ripe plant in it; with `by`,
    /// only what that machine reaches.
    pub const HARVEST: u32 = 1;
    /// `plant`: lift it, dead or alive; with `by`, only if that machine reaches it.
    pub const CLEAR: u32 = 2;
    /// `tray`: wash the perchlorate out of its regolith.
    pub const WASH: u32 = 3;
    /// `tray`, `share`: work organic matter into its regolith.
    pub const AMEND: u32 = 4;
    /// `tray` (or `all`), `spectrum` (`growth` or `inspection`), `dimming` 0..1.
    pub const LAMPS: u32 = 5;
    /// `room`: drain, scrub and refill its nutrient tank.
    pub const CLEAN_TANK: u32 = 6;
    /// `fault` (`power`, `reactor`, `fans:ROOM`, `hvac:ROOM`, `co2:ROOM`,
    /// `leak:ROOM`, `pump:ROOM`, `water_leak:ROOM`, `lamp:TRAY`): break it.
    pub const FAULT: u32 = 7;
    /// `fault`, and `by` a machine that must stand at the fault's service spot.
    pub const REPAIR: u32 = 8;
    /// `room`, and any of `temperature`, `humidity`, `co2`, `pressure`: new setpoints.
    pub const SETPOINT: u32 = 9;
    /// Clean the dust off the solar panels.
    pub const CLEAN_PANELS: u32 = 10;

    pub fn parse(name: &str) -> Option<u32> {
        Some(match name {
            "sow" => SOW,
            "harvest" => HARVEST,
            "clear" => CLEAR,
            "wash" => WASH,
            "amend" => AMEND,
            "lamps" => LAMPS,
            "clean_tank" => CLEAN_TANK,
            "fault" => FAULT,
            "repair" => REPAIR,
            "setpoint" => SETPOINT,
            "clean_panels" => CLEAN_PANELS,
            _ => return None,
        })
    }

    pub fn name(value: u32) -> &'static str {
        match value {
            SOW => "sow",
            HARVEST => "harvest",
            CLEAR => "clear",
            WASH => "wash",
            AMEND => "amend",
            LAMPS => "lamps",
            CLEAN_TANK => "clean_tank",
            FAULT => "fault",
            REPAIR => "repair",
            SETPOINT => "setpoint",
            CLEAN_PANELS => "clean_panels",
            _ => "unknown",
        }
    }
}

/// Which plants a query asks for; `tray` and `species` props narrow it.
#[datapod::datapod(name = "gearbox.greenhouse_query.v1")]
#[derive(Default)]
pub struct GreenhouseQuery {
    /// Only ripe plants when 1.
    pub ripe: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl GreenhouseQuery {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

/// One plant: where it stands in the home frame (x north, y up, z east),
/// how far along and how well it is. Props: `plant`, `tray`, `species`,
/// `stage`, `edible_kg`, `biomass_g`, `water_stress`, `nutrient_stress`,
/// `root_stress`, `mould`, `tip_burn`, `sown_days`.
#[datapod::datapod(name = "gearbox.plant.v1")]
#[derive(Default)]
pub struct PlantRecord {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Development against harvest, 0 sown to 1 ripe.
    pub maturity: f32,
    pub health: f32,
    pub height_m: f32,
    pub alive: u32,
    pub ripe: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl PlantRecord {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

/// One room's air. Props: `room`, `tank_l`, `ec`, `biofilm`, `fans`,
/// `hvac`, `co2_feed`, `leak`, and the power bus's `supply_w`, `demand_w`,
/// `battery_wh`, `panel_dust`, `lamp_power`, `faults`.
#[datapod::datapod(name = "gearbox.greenhouse_room.v1")]
#[derive(Default)]
pub struct RoomRecord {
    pub temperature_c: f32,
    pub humidity: f32,
    pub co2_ppm: f32,
    pub o2_kpa: f32,
    pub pressure_kpa: f32,
    /// Mean light over the room from its lamps, lux.
    pub lux: f32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl RoomRecord {
    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}

/// An action on the greenhouse (`greenhouse_op`), its keys as props.
#[datapod::datapod(name = "gearbox.greenhouse_action.v1")]
#[derive(Default)]
pub struct GreenhouseAction {
    pub op: u32,
    pub _pad: u32,
    #[dp(bytes)]
    pub props: Vec<u8>,
}

impl GreenhouseAction {
    pub fn new(op: u32, props: &Props) -> Self {
        Self { op, props: props.to_bytes(), ..Default::default() }
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.props)
    }
}
