# FEM track contact descriptors

`fem_contacts` version 1 describes collision primitives attached to the tracked
machine's articulated sprocket and passive wheel bodies. Each shape has a unique
`(body, name)` pair, body-local `position` in metres, unit XYZW `rotation`,
`dimensions`, and nonnegative finite `friction`. Every wheel requires geometry;
references to foreign or non-wheel bodies are rejected.

| `kind` | `dimensions` (metres) | Extra field |
| --- | --- | --- |
| `box` | X, Y, Z half-extents | None |
| `cylinder` | radius, Y half-height, unused | None |
| `trapezoid` | X half-width, bottom Y half-width, Z half-height | `top_half_width` |

A trapezoid is extruded along local X. Its bottom edge is at negative Z and
its top edge at positive Z. The four YZ corners are `(-bottom,-height)`,
`(bottom,-height)`, `(top,height)`, and `(-top,height)`. All four size parameters
must lie in `[1e-15, 1e15)` after float32 conversion. A non-null `top_half_width` is required
for this kind and rejected on the other kinds.

```json
{
  "name": "tooth_envelope_00",
  "body": "/ceol/left_sprocket",
  "kind": "trapezoid",
  "position": [0, 0, 0.098],
  "rotation": [0, 0, 0, 1],
  "dimensions": [0.025, 0.013, 0.013],
  "top_half_width": 0.005,
  "friction": 0.6
}
```

The GPU shape layout remains 64 bytes: kind 3 stores the top half-width in
`position.w`; `data.w` retains friction. Molla evaluates the extruded polygon's
distance and normal on the GPU. Flat-face sample gaps use weighted world-space
plane distances; edge and corner gaps use Euclidean distance.

CPU energy-reference exports include the authored GPU descriptors so independent
native checks can reject a changed shape, including its top width. Toothless
causal tests remove named sprocket teeth of either box or trapezoid kind while
retaining smooth wheel supports. Primitive tests and initial clearance are not
proof of sustained torque-driven track motion or usable simulator performance.
