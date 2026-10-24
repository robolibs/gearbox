# Greenhouses

A world USD can hold a plant growth chamber: rooms of air, trays of crops
under LED lamps, nutrient tanks and a power bus. `gearbox-greenhouse` steps
them on world time; the sim draws the crops, lights the rooms, lets sensors
see the plants and serves the whole house on the tool API.

```sh
gearbox run --planet mars --world bin/gearbox/assets/world/mars_greenhouse.usda
gearbox spawn machine bin/gearbox/assets/world/greenhouse_gantry.usda --machine eve --at 12.51 2.74 0
```

`scripts/greenhouse/build_mars_greenhouse.py` writes both files: an EDEN
ISS-like module (porch, service room, a 5.7 m growth room with four racks a
side, a regolith bed, a solar array and a 10 kW reactor on the pad) and the
rail robot that runs under its ceiling. Edit the script and run it again
rather than the USDA.

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
stand at the centres of the places, in the tray prim's frame.

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
`supply_w`, `demand_w`, `battery_wh`, `panel_dust`, `lamp_power`, `faults`.

| Op | Keys | Refused when |
|---|---|---|
| `sow` | `tray`, `species`, `slot`?, `by`? | a place is taken; nothing in reach of `by` |
| `harvest` | `plant` or `tray`, `by`? | a named plant is empty or not ripe, or out of reach |
| `clear` | `plant`, `by`? | the place is empty, or out of reach |
| `wash`, `amend` | `tray`, `share` (0.15) for amend | the tray is not regolith |
| `lamps` | `tray` or `all`, `spectrum` (`growth`, `inspection`), `dimming` 0..1 | |
| `clean_tank` | `room` | |
| `clean_panels` | | |
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
`clean-tank`, `clean-panels`, `fault`, `repair`, `setpoint`.

## 4. Sensing

- Each plant carries a convex hull of its prefab at its stage, rebuilt when
  its look changes. Hulls collide with nothing; depth, LiDAR and recognition
  rays meet them, and recognition names them `plant:TRAY/SLOT`.
- Light sensors count every point and spot light, faded by its range and
  cone, in watts per square metre at daylight's 93 lm/W, and lose the sky
  term when anything stands above them.
- The `climate` sensor kind reads `[temperature °C, relative humidity 0..1,
  CO2 ppm, O2 kPa, pressure kPa]` of the room box it is in, else of the
  planet's air (Mars: 95 % CO2, 0.17 % O2, 0.64 kPa, the sol's temperature
  swing).

## 5. The rail robot

`greenhouse_gantry.usda` is a machine fixed to the world: a rail body held by
a fixed joint, a trolley on the rail (`travel`, 5.0 m), a mast below it
(`lift`, 1.25 m down), a turret at its foot (`turn`, 180° from the S racks
to the N racks) and an arm reaching over a rack (`reach`, 0.65 m). Each
joint has a motor and a `builtin:joint_position` controller of the same
name, so `gearbox machine set-value trolley position 0.5 --machine eve` moves
it half way. The head carries `camera_link` (down, recognition),
`light_link` (up) and `climate_link`. Terrain alignment leaves a machine
with a world joint where it is placed. Retract the arm before travelling:
an arm out over a rack catches its uprights.

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
