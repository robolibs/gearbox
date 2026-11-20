# rungear

Live **Gearbox → Rerun** bridge subscribing strictly over native **`peerbus` shared memory**.

## What it streams
- **Machine poses & trajectories**: `world/<machine>` (`Transform3D` + `LineStrips3D`)
- **Link & TF tree**: static `/links` offsets + live `/tf` joint updates
- **LiDAR**: `Points3D` colored by height or range in the sensor's live TF frame
- **Cameras**: `Pinhole` frustum (`FLU`), RGB `Image`, `DepthImage` (turbo + 3D back-projection), and 2D `Boxes2D` recognition overlays
- **Radar**: 3D target points + radial velocity `Arrows3D`
- **IMU, GPS (`GeoPoints` + scalars), Distance (`Arrows3D` + scalars), Light, Climate, Touch, Position, Compass, Receiver**: live time-series plots & logs

Because Gearbox sensors are demand-driven, closing `rungear` drops its `peerbus` subscriptions and returns sensor GPU cost to zero.

## Usage

Inside `nix develop`:

```bash
# Auto-discover running Gearbox instance and spawn Rerun viewer
./scripts/rungear

# Filter to one machine or specific sensors
./scripts/rungear --machine robot --sensor lidar_link --sensor camera_front_link

# Record directly to a .rrd file (no GUI window)
./scripts/rungear --save /tmp/session.rrd

# Connect to an already-running Rerun viewer
./scripts/rungear --connect rerun+http://127.0.0.1:9876/proxy
```

To install as `rungear` and `gearbox-rungear` (enabling `gearbox rungear`):

```bash
pip install --no-build-isolation --no-deps -e packages/rungear
```
