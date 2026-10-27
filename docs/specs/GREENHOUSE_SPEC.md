# Greenhouses

A world USD can hold plant growth chambers: rooms of air, trays of crops
under LED lamps or glass, nutrient tanks and a power bus. `gearbox-greenhouse`
steps them on world time; the sim draws the crops, lights the rooms, lets
sensors see the plants and serves the whole house on the tool API.

```sh
gearbox run --planet mars --world bin/gearbox/assets/world/mars_base.usda
gearbox spawn machine bin/gearbox/assets/world/mars_base_fleet.usda --machine base
```

The Mars base is a farm worked by robots, sized to what a few cargo landers
could bring. It grows food in four kinds of house: two 24 m glazed domes of
terraced rings lit by the sun, three 30 m inflated arch greenhouses of
tables under LED bars, a printed-regolith vertical farm of 116 LED-lit trays
seven levels high with a nursery, and an EDEN ISS-like container module
with its regolith bed. A soil plant turns dug regolith into growing medium:
it screens it, washes the perchlorate out, dewaters, dries and blends it
with composted crop waste, and bags it. Around the farm stand a habitat
cluster, an inflatable hab, a printed regolith tower, a rover hangar, solar
arrays and four 10 kWe fission units, two container-sized ISRU units with
their tanks and an ice well, and a landing zone. The fleet file places the
robots that do the farm's work (§5), two pressurised rovers and
Perseverance; `scripts/control/mars_farm.py` sets them working.

`scripts/mars_base/` builds everything in Blender (see `build.py`): each
kind of structure goes to `assets/mars_base/modules/<kind>.usdc`, a
`component` model whose looks are baked with Mars wear (dust settled on
what faces up and thrown up low, grime where the sky cannot reach) into
textures of its own in `assets/mars_base/modules/textures/`; the robots go
to `assets/machines/usd/*.usdz`, and the world and fleet files place them. Rocks
lie round the base: Poly Haven's CC0 scans (fetched once into
`.work/mars_base/polyhaven`), boulders as modules with a convex hull to
drive into, stones through one point instancer. Edit the scripts and build
again rather than the USD; `scripts/mars_base/preview.sh OUT module
<file>.<builder> '<kwargs>'` renders one module headless (or `OUT base X Y
RADIUS` a spot of the whole base) to look at first.

Pipes and cables run from tie-in to tie-in, never ending in open ground:
the ISRU manifolds to the tank headers and to the boot seals of Hab2's
utility cabinet, the ice well's water line into the second ISRU, each
Kilopower's lead to a junction box and on to the power station's pit, the
solar field's pit to the station's, and the station's tray to a pit by
HubNorth. The tie-in points are named in the modules (`industry.ISRU_TIE`,
`power.STATION_PITS`, `habitat.tower_feed`, ...) and `layout.py` runs each
line between them clear of the roads, which form one network from the plaza
to the yard, the works and both pads. `scripts/mars_base/clash.sh OUT.json`
lists every two buildings whose meshes meet; collars, roads on the plaza
and parts standing on a pad meet by design, anything else is a misfit.

## 1. USD schema

Every prim with a `gearbox:zone:`, `gearbox:tray:`, `gearbox:power:` or
`gearbox:service:` attribute in a loaded world joins one house. Rooms are read
first, so trays and service spots can name them. Attributes default as listed.

### 1.1 Rooms (`GearboxZoneAPI`)

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `gearbox:zone:name` | token | required | Room name; marks the prim as a room. |
| `gearbox:zone:volume` | float | 30 | Air volume, m³. |
| `gearbox:zone:temperature`, `humidity`, `co2`, `pressure` | float | 21 °C, 0.75, 1000 ppm, 101.3 kPa | Climate setpoints. |
| `gearbox:zone:tank` | float | 300 | Nutrient tank capacity, L. |
| `gearbox:zone:ec` | float | 2.5 | Target conductivity of the tank, mS/cm. |
| `gearbox:zone:centre`, `size` | float3 | centre of a cube of the volume | The box of air in the prim's frame (Z up); climate sensors inside it read the room. |
| `gearbox:zone:workLight` | float | 0 | White work light on the floor, lux. |
| `gearbox:zone:glazing` | float | 0 | Share of daylight the room's glazing lets in to its crops; a glazed room is lit by the sky for cameras too. |

Heating and cooling (6 kW each), dehumidifying, CO2 feed, makeup gas, fans,
wall conductance (22 W/K) and heat capacity (6 MJ/K) are fixed per room.

### 1.2 Trays (`GearboxTrayAPI`)

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `gearbox:tray:species` | token | required | `lettuce`, `tomato`, `potato` or `wheat`. |
| `gearbox:tray:zone` | token | first room | The room it stands in; its tank waters it. |
| `gearbox:tray:size` | float2 | (0.6, 0.4) | Growing area, m, in the prim's X and Y. |
| `gearbox:tray:slots` | int2 | (2, 1) | Plant places in columns and rows; each place holds `area × density / places` plants. |
| `gearbox:tray:medium` | token | `hydroponic` | Or `regolith`. |
| `gearbox:tray:perchlorate` | float | 0.5 | Share of raw perchlorate in a regolith tray; above 0.1 seeds do not sprout. |
| `gearbox:tray:ppfd` | float | 400 | Lamp output at the canopy, µmol/m²/s. |
| `gearbox:tray:lampHours`, `lampOn` | float | the crop's photoperiod, 5 | Hours lit and the hour they come on, on a 24-hour clock stretched over the planet's day. |
| `gearbox:tray:lampHeight` | float | 0.5 | Lamp above the tray, m. |
| `gearbox:tray:sownDays` | float | 0 | Days since sowing at load; 0 leaves the tray empty. |

The lamp is the child prim `lamp`, its emissive face `lamp/face`. Plants
stand at the centres of the places, in the tray prim's frame. A tray under
glass may have no lamp prim and a `ppfd` of 0: it grows by daylight alone.

### 1.3 Power (`GearboxPowerAPI`)

`gearbox:power:reactor` (10000 W), `solarArea` (40 m², 28 % efficient) and
`battery` (40000 Wh) on any prim set the bus. Life support is met before the
lamps; with too little power the lamps dim first.

### 1.4 Service spots (`GearboxServiceAPI`)

`gearbox:service:fault` (`fans`, `hvac`, `co2`, `leak`, `pump`, `water_leak`,
`power`) and `gearbox:service:zone` mark where that equipment is mended. A
machine repairing a fault with `by` must have a part within 1.2 m of the
spot's origin.

## 2. The model

- Crops follow NASA's Modified Energy Cascade (BVAD 2018): canopy closure,
  quantum yield and carbon use in light, CO2 and temperature; transpiration
  from the vapour deficit at total pressure; edible share after the edible
  onset. Stages are seed, seedling, vegetative, forming, ripe, dead.
- Each room is a ledger of moles of N2, O2, CO2 and water with heat: crops
  exchange gas and water with it, HVAC, dehumidifier, CO2 feed and makeup gas
  hold the setpoints, walls lose heat to the outside air, a breach leaks.
- Stresses (water, nutrients, roots, temperature, mould, tip burn) wear
  health down; a plant dies at health 0 or mould 1. Mould grows in still air
  above 85 % humidity and fades once air moves.
- Tanks lose water to roots and leaks, dose nutrients from stock, and grow
  biofilm over weeks that starves roots until cleaned.
- The house steps on world time (`gearbox scene speed`, `gearbox scene skip`)
  in steps of at most 300 s. Mars sunlight on the panels falls with the
  weather's `dust_tau` and the sun's height; dust settles on the panels at
  0.2 % a sol.
- Daylight reaches the crops of a glazed room: the sunlight on open ground,
  at 2.06 µmol of PAR a joule (45 % PAR at 4.57 µmol/J), times the room's
  glazing, on top of its lamps. Crops develop by the daytime mean, which
  follows the weather over two days, and are lit for the sunlit half of the
  day; trays sown at load grow by the same light.
- Dust settles on a glazed room's glazing as it does on the panels, and
  blocks that share of its daylight until `clean_glazing` brushes it off.

## 3. Tool API

| Topic | Mode | Request → answer |
|---|---|---|
| `/gearbox/greenhouse/plants` | que/ans | `gearbox.greenhouse_query.v1` (`ripe`, props `tray`, `species` filters) → one `gearbox.plant_record.v1` per plant |
| `/gearbox/greenhouse/rooms` | que/ans | `Ping` → one `gearbox.room_record.v1` per room |
| `/gearbox/greenhouse/act` | req/res | `gearbox.greenhouse_action.v1` (`op`, keys in props) → `Status` |

A plant record carries its place in its site's frame (x north, y up, z east),
`maturity`, `health`, `height_m`, `alive`, `ripe`, and props `plant`
(`TRAY/SLOT`), `tray`, `species`, `stage`, `edible_kg`, `biomass_g`,
`water_stress`, `nutrient_stress`, `root_stress`, `mould`, `tip_burn`,
`sown_days`, `medium`. A room record carries `temperature_c`, `humidity`,
`co2_ppm`, `o2_kpa`, `pressure_kpa`, `lux`, and props `room`, `tank_l`, `ec`,
`biofilm`, `pump`, `fans`, `hvac`, `co2_feed`, `leak_per_s`, `draw_w`,
`supply_w`, `demand_w`, `battery_wh`, `panel_dust`, `glazing_dust`, `lamp_power`,
`faults`.

| Op | Keys | Refused when |
|---|---|---|
| `sow` | `tray`, `species`, `slot`?, `by`? | a place is taken; nothing in reach of `by` |
| `harvest` | `plant` or `tray`, `by`? | a named plant is empty or not ripe, or out of reach |
| `clear` | `plant`, `by`? | the place is empty, or out of reach |
| `wash`, `amend` | `tray`, `share` (0.15) for amend | the tray is not regolith |
| `lamps` | `tray` or `all`, `spectrum` (`growth`, `inspection`), `dimming` 0..1 | |
| `clean_tank` | `room` | |
| `clean_panels` | | |
| `clean_glazing` | `room`, `by`? | the room has no glazing; `by` more than 1.5 m from the room's box |
| `fault` | `fault`: `power`, `reactor`, `fans:ROOM`, `hvac:ROOM`, `co2:ROOM`, `leak:ROOM`, `pump:ROOM`, `water_leak:ROOM`, `lamp:TRAY` | already broken |
| `repair` | `fault`, `by`? | not broken; `by` more than 1.2 m from the fault's service spot |
| `setpoint` | `room`, any of `temperature` (0..40), `humidity` (0.2..1), `co2` (0..5000), `pressure` (5..110) | |

With `by`, a plant action works only where some part of that machine is
within 0.3 m of the plant's middle (half its height above the tray). A
harvest of a tray takes the ripe plants in reach and answers `harvested` and
`fresh_kg`; fruiting crops keep growing after their fruit is taken.

Everything that happens is a scene event of kind `greenhouse` at the plant's
place, with props `what` (`sown`, `sprouted`, `staged`, `ripe`, `died`,
`harvested`, `faulted`, `repaired`), the plant name as `id`, `species`, and
`stage`, `fresh_kg` or `fault` where they apply.

The CLI wraps all of it: `gearbox greenhouse plants [--ripe] [--tray T]
[--species S]`, `rooms`, `sow`, `harvest`, `clear`, `wash`, `amend`, `lamps`,
`clean-tank`, `clean-panels`, `clean-glazing`, `fault`, `repair`, `setpoint`.

## 4. Sensing

- The plants of a tray within 8 m of a machine's sensor carry a convex hull
  of their prefab at their stage, rebuilt when their look changes, and lose
  it past 12 m. Hulls collide with nothing; depth, LiDAR and recognition rays
  meet them, and recognition names them `plant:TRAY/SLOT`. A house of
  thousands of plants so costs the physics world only what robots look at.
- Light sensors count every point and spot light, faded by its range and
  cone, in watts per square metre at daylight's 93 lm/W, and lose the sky
  term when anything stands above them.
- The `climate` sensor kind reads `[temperature °C, relative humidity 0..1,
  CO2 ppm, O2 kPa, pressure kPa]` of the room box it is in, else of the
  planet's air (Mars: 95 % CO2, 0.17 % O2, 0.64 kPa, the sol's temperature
  swing).

## 5. The farm's robots

Robots on rails are fixed to the world by a joint with an empty body, which
terrain alignment leaves where it is placed. That joint's frame is in the
stage's frame, not the robot's, so a file placing several robots sets each
`Joints/anchor` `localPos0`/`localRot0` where its robot stands (the fleet
file does). Every moving joint has a motor and a controller of its name: a
`builtin:joint_position` one takes a position 0..1 and drives its joint to
position × the moved body's `gearbox:value:range` (negative for a joint
working toward its lower limit), a `builtin:joint_velocity` one a speed.
Both are set on the moved link: `gearbox machine set-value trolley position
0.5 --machine base_fleet_eve`, `... set-value brush velocity 20`. Every
tending head carries `camera_link` (down, recognition), `light_link` (up)
and `climate_link`.

| Robot | Machine | Joints (moved link) |
|---|---|---|
| EDEN's rail robot, after DLR's EVE | `eden_gantry.usdz` | `travel` (`trolley`) 5.0 m along the ceiling rail, `lift` (`mast`) 1.25 m down, `turn` (`turret`) 180° from one rack row to the other, `reach` (`arm`) 0.65 m over a rack |
| An arch's ridge robot | `arch_gantry.usdz` | the same, at 25.6 m, 0.65 m, 180° and 2.4 m: the turret sweeps the aisle under the lamps, the arm telescopes over either row of tables |
| A dome's polar gantry | `dome_gantry.usdz` | `slew` (`boom`) 360° on the cistern, `travel` (`carriage`) 5.6 m out along the boom over the three rings, `lift` (`mast`) 2.2 m down to the canopy; its outer leg runs on the floor rail |
| The vertical farm's stacker crane, after twin-mast miniload cranes | `vertical_crane.usdz` | `travel` (`crane`) 21.9 m along the aisle on a floor rail under a top guide, `lift` (`carriage`) 3.91 m, `turn` (`turret`) 180° from one rack to the other, `reach` (`stage`) 0.72 m: one controller drives the telescope's two stages |
| An arch's harvester: a skid-steer base with a lift column, a UR10e arm (Universal Robots' meshes, BSD-3-Clause) and a soft gripper, cutter and suction tool | `harvester.usdz` | `drive` (`builtin:diff_drive_cmd_vel`); `lift` (`lift_stage`) 0.40 m over two stages; `shoulder_pan` … `wrist_3` (`shoulder_link` … `wrist_3_link`), each turned at its parent to the low end of its working range, so position p is the UR angle lo + p·(hi − lo); `grip` (`jaw_left`). Its `by` reach is measured from `tool/tcp`, between the fingertips |
| An arch's roof washer, after barrel-vault roof washers | `arch_washer.usdz` | `travel` (`portal`) 30.1 m from its dock past the EVA gable, `sweep` (`carriage`) 160° round the vault from curb to curb, `press` (`head`) 0.38 m in toward the cushions, `brush` velocity |
| A dome's glazing washer, after rotating facade gantries | `dome_washer.usdz` | `slew` (`arch`) 360° on the ring girder round a crown bearing, `meridian` (`carriage`) from 82° to 9.5° elevation, `brush` velocity |
| A solar row's washer, after Ecoppia's waterless robots | `panel_washer.usdz` | `travel` (`frame`) 21.1 m along the row on rails at its top and bottom edges, `brush` velocity; its own panel charges it |
| The soil plant's excavator, after NASA's RASSOR 2.0 | `regolith_excavator.usdz` | `drive`; `arm_front`, `arm_rear` (position 0 stowed, 0.33 over a bin's lip, 0.40 carrying, 0.787 drums on the ground, 1 lifting the chassis onto them); `drum_front`, `drum_rear` velocity, digging at ±2.1 rad/s and emptying turned back |

The fleet spawns them as `base_fleet_eve`, `base_fleet_arch{a,b,c}bot`,
`base_fleet_dome{a,b}bot`, `base_fleet_farmcrane`,
`base_fleet_arch{a,b,c}picker{1,2}` (one each side of an arch's walkway, its
arm toward its row of tables), `base_fleet_arch{a,b,c}washer`,
`base_fleet_dome{a,b}washer`, `base_fleet_solarwasher{1..4}` and
`base_fleet_digger{1,2,3}` (on their docks at the soil plant). Retract an
arm before travelling: one out over a rack catches its uprights. A `by`
reach is measured from the parts under the named robot's own prim.

The build writes the farm's work plan next to the fleet,
`bin/gearbox/assets/world/mars_base_farm.json` (`scripts/mars_base/plan.py`):
for every robot that tends plants, the joint positions bringing its head
over each plant place it reaches (a harvester's also where it stands in its
lane), the order its joints move in and where it parks; for the washers
their sweeps, for the excavators their dock, dig ground and bins.
`scripts/control/mars_farm.py` works the farm by it: each tending robot
harvests what is ripe and sows what is empty, the washers sweep and then
clean the glazing with `by`, the excavators dig and tip into the soil
plant's bins. It waits on the sim's clock, so it keeps pace however fast the
sim runs.

The fleet's `base_fleet_rover1` and `rover2` are NASA's Space Exploration
Vehicle (`assets/machines/usd/sev_rover.usdz`), its full-detail mesh cut into the
chassis and six wheel modules. Each module is an arm sprung on the chassis
(`arm_*`, ±12°), an upright steering on it (`steer_*`, ±60°) and a pair of
twin wheels (`spin_*`). A six-wheel ackermann `cmd_vel` controller turns the
front and rear modules opposite each other about the middle axle
(`gearbox machine move` or `drive`). Its textures carry baked Mars wear
(`scripts/mars_base/weathering.py`): dust settled on what faces up and
thrown up by the wheels, grime where the sky cannot reach, rubber treads,
and roughness and metal read from NASA's colour and specular maps. It has
a front camera, a roof LiDAR and an IMU.

`base_fleet_percy` is NASA's Perseverance (`assets/machines/usd/perseverance_nasa.usdz`),
working the ground south-east of the base: the base stands in Jezero crater.

## 6. Scenarios

Measured on the generated greenhouse at day 150, from a fresh load:

| Scenario | Commands | What follows |
|---|---|---|
| Power cut | `fault power`, `scene skip 4h` | Lamps dark (0 lux, light sensors 0); 21 → 18 °C, CO2 1000 → 1340 ppm, humidity 75 → 89 % |
| Dust storm without the reactor | `fault reactor`, `scene weather dust_tau=4`, `scene skip 18h` | Battery 40 → 0 kWh; lamps shed to 42 % while the rooms hold 21 °C |
| Fan failure | `fault fans:grow`, `scene skip 3sol` | Mould 0 → 0.24 on every plant, health 0.83; mended within days, it fades |
| Breach | `fault leak:grow` | Makeup gas holds the pressure; with makeup failed the room falls below 10 kPa in 3 days |
| Biofilm | `scene skip 30sol`, `clean-tank grow` | Biofilm reaches 1 and starves roots; cleaning drops it to 0.12 two sols later |
| Perchlorate | `sow RegolithBed potato`, `skip 12sol`, then `wash`, `amend`, `sow` | Raw regolith: seeds after 12 sols; washed and amended: seedlings |
| Robot repair | `repair fans:grow --by eve` | Refused at 2.06 m from the air handler; done with the trolley at the rail's north end, under it |
| Robot harvest | `harvest TrayN20a --by eve` with the head at the tray | The 4 ripe lettuces within 0.3 m, 1.0 kg |
