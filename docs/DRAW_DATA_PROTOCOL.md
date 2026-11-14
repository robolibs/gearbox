# Typed spatial data contract

Implementation checkpoint: Releases A and B, 2026-11-13. Six dataset kinds are retained
and rendered: `cloud`, `octree`, `surfels`, `mesh_blocks`, `volume`, `elevation`. The
user guide is [DRAW_DATA_GUIDE.md](DRAW_DATA_GUIDE.md). The paragraphs below up to
"Request deadlines" are the Phase 4 record, kept as written.

**Implemented:** Rust typed schemas, per-scan capture descriptors, occupancy keys,
binary page codecs, checked envelopes and operation metadata, companion topic
registration, capability negotiation, a synchronous Rust data connection, a generic
identity facade, bounded list/status reply contracts, optional native capture props,
lossless CLI JSON descriptors, charged packed pages,
immutable manifests, a bounded transaction owner, shared native legacy/data ownership,
bounded live listing/status, lifetime maintenance, presentation-only publications and
whole-scan accumulation with simulation/source age policies, compact native cloud
rendering, exact completed-frame receipts and optional source-frame clipping boxes.
Since then, renderers for all six kinds and the `gearbox draw data` CLI controls
(layer, slice, field) have landed. The viewer advertises `rendered == retained`. API-only
hosts advertise no rendered kinds and report `unavailable`. `gearbox.draw.v1` is
unchanged. Whole-frame client coalescing is not implemented. Gate results are in
`DRAW_DATA_ACCEPTANCE.md`.

## Request deadlines

Rust `Client::draw_data`, `draw_data_at`, `draw_data_capabilities` and
`DrawDataClient::request` default to **five seconds**. Their `_with_timeout` and
`_with_deadline` variants accept a `Duration` or absolute monotonic `Instant`.
Negotiation shares one deadline across signed directory lookup, capability retrieval
and construction of all data routes.

The deadline is checked before each request is published and after its reply arrives.
Each request opens its own requester on the negotiated route, so other clients' replies
on the shared ring never leave an idle requester behind. Requests use the pinned
peerbus `call`: a local shared-memory call gives up after peerbus's own five-second
default; a remote call has no transport timeout.
`DrawDataError::Timeout` is distinct from unsupported schema/topic and host rejection.

**A delivered mutation can still complete after timeout.** Do not assume rollback or
start an automatic replacement transaction. Query status or retry the same idempotent
transaction operation with the same identity/bytes; an OPEN_WRITER timeout may consume
a lease until its bounded expiry. These low-level clients do not automatically retry.

## Bounded Rust request pipeline

`Client::draw_data_pipeline(config, connection_deadline)` negotiates one host epoch
and starts a fixed set of native request workers. `draw_data_pipeline_at` accepts an
explicit host. Defaults are four workers, 64 outstanding requests, 32 MiB and five
seconds per admitted request; the advertised host client-queue limit can impose a
smaller cap. Configuration permits 1–8 workers, at most 1,024 operations, and a
positive request timeout no greater than 60 seconds.

```rust,ignore
let pipeline = client.draw_data_pipeline(DataPipelineConfig::default(), deadline)?;
let mut ticket = pipeline.submit(&request, deadline, DataAdmission::Wait)?;
let reply = ticket.wait(deadline)?;
// Check reply.code and reply.stage before treating any data as retained.
```

`DataAdmission::Reject` returns `Full` instead of waiting. Wait admission borrows
caller-owned input and copies it only after byte/operation reservation. It does not
retain uncharged payload copies for blocked producers. The queue conservatively
charges input Vec capacities, fixed queue storage, bounded retained replies/errors,
and a 2 MiB serialization/validation allowance per worker. Caller-owned source data,
transport runtime/SHM mappings and thread stacks are outside these queue figures;
they are not a process-RSS claim. Envelope bounds/flags/epoch are checked at admission;
full typed metadata validation runs on the fixed workers before publication.

Only consecutive CHUNK requests overlap. Every other request is a FIFO barrier before
and after active work, including declarations, COMMIT, CANCEL, DELETE and CLEAR.
Admission establishes order; concurrently waiting producers have no defined invocation
order. No request is coalesced. A transaction owner must await all chunk results and
check their codes before submitting COMMIT. Queue admission is not retention or visibility.

Unconsumed completed tickets still occupy bounded operation slots and reply storage.
`try_take`/`wait` transfer result ownership; dropping a ticket releases its result when
the operation finishes, but does not silently cancel its mutation. `cancel` removes a
queued request and frees its payload, or explicitly reports `InFlight`/`Complete`.
Queued deadlines expire without publication; a deadline after dispatch conservatively
reports `may_have_been_sent: true`. A wait timeout alone does not cancel the request.
Transport/protocol failures can likewise leave a delivered mutation's outcome unknown.

`shutdown(&self)` closes admission, wakes waiting producers and cancels queued requests.
`close(&self)` additionally joins active bounded workers; Drop calls close. Neither
request cancellation nor pipeline shutdown automatically cancels a whole host transaction
or closes its writer lease. The transaction owner must send CANCEL/close the lease;
`DrawDataWriter` below supplies this lifecycle for synchronous borrowed-page submissions.
Whole-frame client coalescing remains separate pending work.

A request refused a slot on a full request ring was never published; it is retried
until its deadline. A request is never resent after publication, and other errors are
not converted into retries.
Native concurrency, admission, barrier, cancellation and retained-upload checks run
with `test-draw-pipeline`, also included by `test-draw-stream`.

`measure-draw-data-capacity --args='fake|INSTANCE POINTS REPETITIONS'` compares direct
sequential retained uploads with fixed-worker windows 1/4/8, checks exact retained counts,
and cleans only its process-owned namespace. It reports local SHM retained throughput,
not cross-machine, rendering, visibility, process memory, or sensor-rate acceptance.

## Borrowed-page Rust writer

`pipeline.writer(chunk_window, deadline)` acquires a host writer lease. The window
is 1–64 and cannot exceed the pipeline's operation cap. Each live writer reserves
2 MiB of conservative metadata/chunk/ticket workspace from the same queue budget;
admission also leaves room for its acquisition request/result. The reservation is
released on writer destruction. Source pages and descriptor slices remain borrowed
caller-owned input, not a second full payload copy.

```rust,ignore
let pages = [DataUploadPage { descriptor: &page_descriptor, bytes: &encoded_page }];
let scans = [scan_descriptor];
let mut source = DataUpload::replace(&descriptor, &scans, &pages);
source.base = previous_revision;
let mut writer = pipeline.writer(8, connection_deadline)?;
let retained = writer.upload(&source, upload_deadline, Some(&cancel_flag))?;
writer.close(cleanup_deadline)?;
```

`DataUpload` also supports `CommitMode::Delta`, exact base revisions, removed page
addresses and removed scan keys. The declared counts/total bytes are derived from
the borrowed input. Pages are declared and divided into bounded 12 KiB chunks; every
chunk reply must be successful and identify the same writer/transaction before COMMIT.
No full submission is buffered or silently dropped. The mutable writer permits one
submission at a time; multiple writers share the pipeline's byte/operation caps.

All submission operations share one absolute deadline; the pipeline request maximum
can shorten an individual call. Cancellation is cooperative between calls/chunks and
does not preempt an already delivered native request. On failure, queued chunk tickets
are explicitly cancelled before a CANCEL barrier is attempted with an additional,
bounded **250 ms** cleanup budget. There is no automatic mutation/transaction retry.

`DataUploadError` preserves the primary error, host epoch, writer lease, transaction
number and a separate cleanup outcome:

- `NotStarted`: local validation/cancellation/deadline prevented starting this upload.
- `Cancelled`: the host acknowledged cancellation without a retained revision.
- `Retained(revision)`: cancellation discovered that COMMIT already succeeded, e.g.
  after an invalid/lost acknowledgement. This is not rollback or a request to retry.
- `Unconfirmed(error)`: cleanup failed or did not acknowledge a terminal outcome.
  Query the same identity; do not assume the old/new generation or lease is gone.

A deadline's `may_have_been_sent` flag describes that failing operation, not all prior
operations in the upload. Likewise cancellation requests do not prove rollback without
the separate cleanup acknowledgement. A successful upload returns the actual `DataReply`
with its retained/visible fields; retention alone never claims rendering.

Explicit `close(deadline)` reports lease-close failure and does not delete retained
datasets. Drop attempts close within 250 ms but cannot report errors; server lease expiry
is the fallback, including after a lost OPEN_WRITER reply. Keep the writer alive and
use explicit close when cleanup confirmation matters. Tests run via `test-draw-writer`
and the stream gate; the live probe exercises public writer replacement/delta/cleanup.

`Arc<DrawDataPipeline>::writer_owned` returns a writer that retains its pipeline without
self-referential borrows. Dropping the original Arc does not invalidate the writer;
explicit pipeline shutdown still closes admission for every shared owner.

## Transaction ownership

Page decoding and generation validation run on the owned drawing service thread,
outside the shared registry mutex. Publication pins its candidate until after unlock,
including rejected commits; payload destruction is not part of the publication lock.
The deterministic `test-draw-concurrency` gate checks reads/clear during both cloud
and occupancy validation, rejected-candidate destruction, and listing serialization.

`gearbox_api::draw_data::StreamingStore` accepts checked `DataRequest` packets and
monotonic milliseconds. FakeHost and the Bevy drawing service share a charged
`DrawingBackend` with their legacy drawing route. Only one transaction owner can
attach to a backend at a time; cloning a backend does not multiply service quotas.
It never acknowledges visibility. Successful chunks/declarations are `received`;
publication is `retained` with an exact incarnation/generation. COMMIT retries return
the recorded outcome and never republish. Cancellation retries are distinct from commit.

Transaction IDs must strictly increase for each new BEGIN within a writer lease.
Retries use the same ID and byte-identical BEGIN metadata, declared total and base.
Conflicting pending declarations or failed uploads terminate the submission and release
its pages/reservations. Wrong object/base addresses cannot mutate a different submission.
Bounded completion-history eviction yields `history_expired`, not ID reuse. Early memory
admission rejection can consume an ID without retaining a result; use a new ID afterward.

DELETE/CLEAR revoke matching publication permits and conservatively expire all completion
history at the next service maintenance step, including completed failures/cancellations.
Unrelated pending transactions remain valid. Shared legacy-registry barriers follow the
same path. Host restart changes the host epoch; old leases/transactions cannot be reused.

Delta generations retain schema, octree frame/configuration and stream policy. Latest
submissions replace complete frames; an accepted new pending frame supersedes the previous
pending one only after base validation and reservation. The current complete frame remains
retained. Acquired scan metadata cannot change in place; new scan keys exceed the committed
incarnation high-water mark. Removing scans or publishing an empty replacement does not
reset it. Deleting/recreating the dataset creates a new incarnation.

The CPU ledger counts declared reservations, immutable payloads, upload indexes, metadata
and construction scratch. Incomplete-page quotas release only when a page is validated;
the staged payload still counts against the writer and total budgets. Registry index
charges outlive the service if a reader retains the registry. Conservative table allowances
are not allocator/RSS measurements. Deletions enqueue retired generations in a charged,
preallocated queue; the service drops them outside the registry mutex. Queued retirement
continues to occupy an identity slot until drained. Reader pins keep page charges alive.

Mixed listing seeks exact namespaces and slash-delimited descendants in the ordered
registry; unrelated objects are not scanned. Each lock captures at most one bounded
summary. JSON validation/encoding and cursor construction occur after unlock. All
summaries must share one listing revision: concurrent mutation returns `stale_cursor`,
including on the first page, rather than silently returning a mixed snapshot. Restart
the query without a cursor after that error. Item/byte caps and oversized legacy-key
errors remain explicit; legacy identities are not truncated to fit typed summaries.

Object lifetime starts at publication in simulation seconds. Paused simulation does
not expire objects even though writer leases still expire in monotonic transport time.
Presentation updates preserve the original lifetime start. Simulation rewind cancels
pending publications and removes acquired or lifetime-bound datasets; permanent retained
maps remain. Marker/all scene reset removes both legacy and typed drawings. Machine/prop
reset does not implicitly remove drawings.

## Whole-scan accumulation

`Accumulate` validates the complete candidate before applying retention. Old scans cannot
hide malformed pages, duplicate source IDs, wrong capture domains or incomplete source
counts. Construction reserves the bounded union of one retained generation and one
declared transaction, up to twice the retained scan/page limit in charged scratch/reference
tables. The final generation still obeys negotiated limits. Retained maps are not evicted.

Scans are ordered by the selected capture clock, then by scan key for equal timestamps.
Age expires at `age >= max_age_s`. Count and payload-byte pressure remove whole oldest
scans, never individual points/pages. Surviving pages and capture metadata retain shared
identity. A scan larger than the entire payload budget rejects the submission with
`budget`, preserving the previous complete generation rather than emptying it for a scan
that cannot fit. Explicit caller removals/replacements are not policy eviction.

- **Simulation age:** elapsed simulation seconds since each capture. Pausing freezes
  age; source timestamps do not affect it. Service maintenance publishes an atomic
  new generation when scans expire, without resetting object lifetime. Pending deltas
  against the previous exact base then receive a conflict, never silently merge.
- **Source age:** the highest successfully committed source timestamp minus capture
  time, in the same named clock and scene epoch. Exact `u64` nanoseconds are subtracted
  before converting the difference to seconds, including near `u64::MAX`. Wall/simulation
  time does not advance this watermark. Late captures cannot rewind it.

A charged metadata anchor retains the watermark/domain after scans disappear, including
empty replacement and presentation changes. Under the same accumulation age-clock policy,
changing capture scene epoch or source-clock name requires delete/recreate. Explicit
replacement to another stream/age-clock policy resets the clock anchor but not the
scan-key high-water mark. Simulation rewind removes acquired datasets as described above.

Policy removal of a previously retained candidate increments `evicted_scans`. A submitted
scan excluded before first retention increments `dropped_scans`. Atomic commit retries
do not repeat counters; rejected/stale publications do not count uncommitted evictions.
These cumulative counters remain host-wide, including in per-key status responses.

Age maintenance reserves new reference tables before publication. If memory or publication
permits are exhausted, the old complete generation remains retained and status reports a
bounded `retaining` failure. Records are tied to the failed revision and pruned on mutation
or deletion. Maintenance retries after allocation usage or permit count changes, or one
simulation second; unchanged failures do not flood the log. Status selects the most recent
transaction/maintenance failure. Releasing a permit can therefore recover expiry even when
the simulation is paused. Reader-pinned old generations remain charged after eviction.

## Cloud schema revision 1

API location: `gearbox_api::wire::draw_data`.

`CloudSchema { revision, attributes }` uses bounded UTF-8 JSON for its descriptor,
not for bulk arrays. `from_json` rejects more than 8 KiB before JSON decoding and
validates revision, fields, and semantic types afterward. Unknown JSON fields,
unknown enum values, duplicate attribute names, and unsupported revisions fail.
Public local constructors/Serde decoding are not a substitute for this bounded
network-entry method.

A schema has at most 32 attributes, exactly one position attribute, and exactly one
source-ID attribute. Names are unique, nonempty, at most 63 UTF-8 bytes, and contain
no control characters. Units are bounded to 63 bytes without control characters.
Every descriptor explicitly provides `name`, `scalar`, `components`, `semantic`,
`units`, `normalization`, and `nullable`.

Scalar encodings are little-endian `u8`, `u16`, `u32`, `u64`, `i16`, `i32`, `f32`,
or `f64`, with 1–4 components per sample. Columns are tightly packed with no alignment
padding. Checked multiplication computes expected byte counts before sample access.
Normalization metadata preserves raw integer bytes; validation never changes them.
`unorm` supports u8/u16/u32; `snorm` supports i16/i32; floats and u64 use `none`.

| Semantic | Required representation |
|---|---|
| position | Three f32/f64 components, metres, no normalization |
| source_id | One u64, nonnullable, no normalization; 0 and u64::MAX are valid IDs |
| color | Four u8 RGBA components, unorm |
| intensity | One component; caller-declared units/range, not forced into 0..1 |
| category, ring, return_index | One u8/u16/u32 component, no normalization |
| confidence | One f32/f64 component in 0..1 |
| normal | Three f32/f64 components; squared length within 0.002 of one |
| radius | One positive f32/f64 in metres; pixel-size presentation remains separate |
| acquisition_offset | One u64 in nanoseconds; capture clock belongs to the scan descriptor |
| custom | Any supported scalar/component count and compatible normalization |

RGBA channel order is byte R, G, B, A; it is not a host-endian packed integer.
Color-space declaration and color-selection/palette policy are explicit presentation
metadata. Validation does not reinterpret RGBA or apply a theme. Native compact-cloud
color mapping is described below; unsupported representation renderers remain pending.

## Column and page payloads

`ColumnData` contains `values: Vec<u8>` and `validity: Vec<u8>`. `CloudPage` contains
`count` and columns in exact schema order. A page is limited to 65,536 source samples
and 32 MiB total value/mask bytes. Clients must choose fewer samples per page when
large optional attributes reach the byte cap first. This page cap is distinct from
the future transport chunk limit, aggregate ownership budget, and visible-point cap.

- An empty mask means all samples are valid.
- A populated mask is permitted only for a nullable field; its length is exactly
  `ceil(count/8)`, bit zero denotes the first sample, and unused final bits are zero.
- Invalid samples have canonical all-zero payload bytes, not NaN or infinity.
  Ingest adapters must normalize source missing-value conventions before validation.
- Every valid floating sample is finite. Semantic radius/confidence/normal constraints
  apply only to valid samples.
- Position validity does not remove the corresponding source ID or other attributes.
  Renderer compaction must preserve that mapping.
- Source IDs are checked for uniqueness within each page. The immutable generation
  builder also validates uniqueness across every page of a scan using charged sort scratch.
- A zero-sample page has the declared number of empty columns and no mask bytes.

`CloudPage::validate` checks schema/count/column cardinality and total bytes before
walking samples. The source-ID uniqueness set is bounded by the page count; its
scratch allocation must be included in the future store reservation budget.

## Decisions and evidence

**What:** Added a typed, little-endian cloud data model with semantic validation,
explicit validity, source IDs, bounded descriptors, and independently retained columns.
**Why:** The legacy fixed draw sections cannot represent simultaneous rich attributes
or preserve organized invalid observations without overloading their meaning.
**Where:** `crates/gearbox-api/src/wire/draw_data/`, `tests/draw_data.rs`, `.make.lua`.
**Learned:** Raw sample layout, scientific validity, and page byte limits must be
defined before transport or renderer shortcuts; transport compression alone is not
a GPU representation.

```sh
nix develop --offline --command oslo make test-draw-data
```

The earlier page-contract checkpoint covered 12 schema tests, 15 descriptor/frame/presentation
tests, 9 envelope tests, 10 page codec tests, 7 occupancy tests, and 7 fixture tests.
The recipe now also covers operations, capabilities, actual host/client negotiation,
identity, capture, status, JSON/CLI and independent wire fixtures. Current results
are in `DRAW_DATA_IMPLEMENTATION.md`. None establish incremental streaming, native
representation rendering, or the full plan complete.

## Dataset and per-scan descriptors

`DataDescriptor` contains revision, key, shape, stream policy, optional simulation
lifetime, and presentation. Cloud shape owns a schema; octree shape owns configuration
and its frame. Cloud placement does **not** live in a mutable dataset-wide frame.
Bounded dataset JSON is at most 12 KiB, separately from each scan's 4 KiB JSON.

`ScanDescriptor` contains revision, scan key, optional source identity, `DataFrame`,
source count, and optional width/height organization. Dimensions include invalid
observations and must multiply to the source count. Empty dimensions are 0 by 0.
Static retained clouds use scan key zero. Latest/accumulated acquired frames require
nonzero keys and capture stamps. One committed latest snapshot has at most one scan;
accumulation has the configured cap, at most 4,096 scans. The transaction owner enforces
key non-reuse across commits; whole-scan eviction still requires runtime policy integration.

`validate_scans` rejects duplicate keys, over-limit tables, epoch mixing during
accumulation, and mixed named clocks for source-age accumulation. It does not yet
validate cross-page IDs or page-to-scan counts; those are commit-time requirements.
The generation builder now checks those cross-page IDs and exact page-to-scan counts.
The same source/pulse ID may exist in different scans, never twice within one scan.

Capture stamps include finite nonnegative simulation seconds, an optional paired
u64 source timestamp/named clock, original sample, and nonzero scene epoch. Source
nanosecond addition uses checked integer arithmetic, including near `u64::MAX`.
Snapshot placement requires an exact matching source frame and capture stamp plus a
rigid source-to-target pose and anchored target datum. A live-link target is invalid.
The datum field currently remains optional in Rust/JSON shape, but snapshot validation
requires it; omission is not an identity-datum fallback.

The f64 CPU transform oracle applies dataset origin plus page origin plus local
offset, then dataset pose, then capture/live source-to-target pose. `captured_ecef_point`
applies the captured datum, never a newly resolved target frame. Unit wxyz quaternions,
finite inputs and outputs are checked; directions rotate without translation. These
helpers test the contract, not integration into the existing world renderer or sensor.

Presentation declares constant/RGBA/scalar/category/axis/range/age colors, occupancy
state/probability/depth palettes, explicit pixel/metre radius, optional metric radius
attribute, opacity/glow/blur, and bounded octree display settings. Age selects its
clock explicitly. Attribute selectors retain coexisting columns. Unknown octree views
require a finite ROI and a 1–1,000,000 cell budget. The compact cloud renderer consumes
the cloud presentation fields; typed occupancy rendering remains pending.

## Occupancy spatial keys

`OctreeConfig` fixes positive finite finest resolution, depth 0–21, and ordered
free/occupied probability thresholds in 0–1. A root spans resolution times 2^depth.
`VoxelKey` contains signed i64 tile XYZ, depth, and u64 Morton path. Each path digit
uses x/y/z as bits 0/1/2; the first level is the most significant used digit. Unused
high bits are zero. Finest-grid bounds use i128 arithmetic; negative coordinates use
Euclidean floor division. Growth does not renumber existing tiles.

Unknown probability is `None`, not zero. Threshold equality is free at `free_max`
and occupied at `occupied_min`; between them is known uncertain. Confidence and class
are independent. Leaf-set validation rejects duplicate keys and ancestor overlap in
either order. This is snapshot validation; incremental split/merge storage, aggregates,
unknown-space expansion and rendering are not implemented.

## Binary pages

All integers/floats are little-endian; there is no native alignment padding. Page
addresses are `(scan_key, page_id)`. Occupancy pages use scan key zero and zero page
origin because positions come from spatial keys. Cloud page origins are finite f64
metres, kept in `PageDescriptor`, not repeated inside column payloads.

### Cloud columns v1

| Offset | Encoding |
|---|---|
| 0 | Four magic bytes `GBC1` |
| 4 | u32 source sample count |
| 8 | u32 column count |
| 12 | u32 reserved zero |
| 16 | One pair of u32 value-byte length and validity-byte length per schema column |
| After table | Each column's values then validity mask, in schema order |

The decoder validates header/count/table lengths, checked total sizes, and exact end
of payload before copying columns. Then it applies the existing semantic validator.
Encoded maximum is 32 MiB plus 16 header bytes plus at most 256 table bytes. Scratch
and decoded copies are not free: the future store must reserve them in its CPU budget.

### Occupancy leaves v1

Header: `GBO1`, u32 leaf count, eight reserved zero bytes. At most 65,536 leaves/page.
Each record is exactly 48 bytes:

| Record offset | Encoding |
|---|---|
| 0–23 | Three signed i64 root tile coordinates |
| 24–31 | u64 Morton path |
| 32 | u8 depth |
| 33 | Presence bits: probability=1, confidence=2, class=4; no other bits |
| 34–35 | Reserved zero |
| 36–39 | f32 probability |
| 40–43 | f32 confidence |
| 44–47 | u32 class |

Absent attribute payload bytes must be all zero. Present zero values remain distinct
from absence. Exact lengths, flags, keys, probabilities and topology are validated.
Truncation, extra bytes, NaN and noncanonical absent values are rejected.

### Mesh blocks v1

One indexed triangle block per page (`MeshBlockV1`). `items` is the triangle count.

| Offset | Encoding |
|---|---|
| 0 | `GBM1` |
| 4 | u32 vertex count, 3..65,536 |
| 8 | u32 index count, a nonzero multiple of 3, at most 131,072 triangles |
| 12 | u32 flags: normals=1, colours=2, confidence=4 |
| 16 | f32 xyz positions, then f32 xyz normals, RGBA8 colours and f32 confidence when flagged, then u32 indices |

Indices stay below the vertex count, normals are unit length, positions finite,
confidence 0..1. A delta replaces or removes whole blocks by page id.

### Volume blocks v1

One sparse block of `side³` samples per page (`VolumeBlockV1`); `side` is 8 or 16, so
`items` is 512 or 4096. The page id packs the signed block key, 21 bits per axis
(`volume_page_id`); keys reach ±2^20.

| Offset | Encoding |
|---|---|
| 0 | `GBV1` |
| 4 | three i32 block key |
| 16 | u32 block side |
| 20 | u32 flags: weights=1, colours=2 (must match the field) |
| 24 | f32 values, then the validity bitmask (bit i of byte i/8), then f32 weights and RGBA8 colours when flagged |

Sample `i = x + side·(y + side·z)`. Invalid samples hold canonical zeros in every
array. A block never sent is unallocated, which is distinct from an unknown sample.
`VolumeField` gives the kind:
- `Tsdf { truncation_m }`: weights are required;
- `Esdf { signed }`;
- `Scalar { units, range }`.
It also gives `voxel_m`, `block_side` and `colors`.

### Elevation tiles v1

One layer of one tile per page (`ElevationTileV1`); `items` is `side²`, 64..16,384. The
page id packs tile x and y (24 bits each) and the layer (bits 48+); layer 0 is
elevation and layers 1..15 follow `ElevationGrid.layers`.

| Offset | Encoding |
|---|---|
| 0 | `GBE1` |
| 4 | two i32 tile key |
| 12 | u32 layer |
| 16 | u32 tile side, 8..128 |
| 20 | f32 values, then the validity bitmask |

Cell `i = x + side·y`. A missing elevation sample is a hole in the topology. A missing
sample in another layer only removes that colour.

`PageDescriptor` declares encoding, address, item count, byte count, origin and CRC.
CRC is CRC-32/ISO-HDLC over the full encoded page; the `123456789` check vector is
`0xcbf43926`. `crc32fast` 1.5.2 was already locked transitively and is now a direct API
dependency. Descriptor decoding verifies byte length and CRC before page parsing;
integrity checks provide no authentication. Golden Rust and CLI byte tests exist.

## Companion envelopes

Canonical datapod identities are `gearbox.draw.data.request.v1` and
`gearbox.draw.data.reply.v1`, distinct from unchanged `gearbox.draw.v1`.

Request fixed fields: revision/operation/flags/reserved as u32, then host epoch,
lease, transaction, incarnation, base generation, scan key, page ID, offset and
total bytes as u64. Sections are `key`, `metadata`, `payload`. Key is bounded JSON
(1 KiB); metadata is bounded JSON (12 KiB); chunk payload is at most 12 KiB.
Flags/reserved are zero. `pack_checked` tests the actual datapod wire size against
the 16 KiB host ceiling. `unpack_checked` checks that ceiling before decoding and
rejects noncanonical padding/trailing encodings by canonical reserialization.

Operations 0–15 in order: capabilities, open writer, close writer, begin, declare scan,
declare page, chunk, remove page, remove scan, commit, cancel, patch presentation,
delete, clear, list, status. Routing validation requires the appropriate object key,
host epoch, lease, transaction, scan/page address, metadata and payload presence.
Only chunks have offset/payload; only BEGIN declares total transaction bytes.
Chunk range addition is checked. BEGIN metadata declares descriptor, replace/delta,
scan/page declarations and scan/page removals. Counts are bounded to 4,096 scans and
65,536 pages per change set; replacement cannot carry removals.

Reply fixed fields: u32 revision/code/stage/reserved, then u64 host epoch, lease,
transaction, incarnation, stored generation, visible generation, view ID and view
epoch. JSON metadata is at most 12 KiB; UTF-8 diagnostic text is at most 512 bytes,
without control characters. Stages are none=0, received=1, retained=2, visible=3.
Retained stages require identity/generation; visible stages additionally require a
specific view and nonzero visible generation. Error codes cover unsupported, invalid,
conflict, budget, expired, not found, stale epoch, superseded, visibility unavailable,
view gone, history expired, internal, and stale cursor.

**Important integration boundary:** envelope validation is not operation validation.
The data lane uses checked unpacking and typed intrinsic metadata validation. BEGIN
requires matching keys, paired incarnation/base generation, an existing delta base,
and total bytes within the declared page-count bounds. Scan/page declarations must
match the header address. Contextual descriptor/store validation, effective quota
enforcement and generation comparisons still require the store, not merely parsing.
The legacy generic `serve_req` path alone does not provide these gates. Exact per-view
completion, supersession, retries, quotas and historical results remain pending.

## Running companion service and Rust negotiation

HostBus registers four request/response topics with unchanged shared-memory limits:

| Topic | Allowed operations |
|---|---|
| `/gearbox/draw/data/capabilities` | capabilities |
| `/gearbox/draw/data` | open/close writer, BEGIN, declarations, chunks, removals, commit/cancel, presentation, delete/clear |
| `/gearbox/draw/data/list` | list |
| `/gearbox/draw/data/status` | status |

FakeHost and DrawServicePlugin poll the lane. Each poll processes at most 16 requests
per route, capabilities/status/list before mutations. Handlers and metadata decoding
run outside transport locks. Every process gets a random nonzero host epoch; stale
requests return `stale_epoch`. Malformed packets, wrong routes and invalid typed
metadata return bounded `invalid` replies. Invalid handler output becomes `internal`,
never a false retained/visible acknowledgement.

Current capabilities expose schema revision 1, decodable/retained cloud and octree
formats, atomic generations, deltas, presentation updates, unified listing and accumulation.
`rendered` remains empty; picking and per-view completion remain false.
Limits describe configured allocation ceilings, not measured RSS or throughput. Subbudgets
must fit their parent budgets; declared schema lists are unique, nonzero, and at most
16 entries. Future-only schema lists are valid messages but fail client negotiation.

`Client::draw_data_capabilities()` queries the selected host. `Client::draw_data()`
returns `DrawDataClient`; bare clients use `draw_data_at(endpoint)` explicitly. Its
`request(&DataRequest)` checks metadata, epoch and real packed size before one
synchronous call. Error replies remain explicit `DataReply` codes; transport,
protocol, unsupported negotiation and capability rejection are distinct Rust errors.
It has no background submission queue or retries; requests have the native deadlines
documented above. The separate pipeline adds bounded asynchronous admission.

`test-draw-data` covers CPU contracts and twelve native host/client cases without a
GPU. `test-draw-live --args=new` additionally runs `draw_data_probe` against the
actual Bevy simulator and then checks legacy drawing operations. The probe
retains true cloud and occupancy pages and checks retries, deltas, shared ownership,
listing, status, presentation, terminal corruption failures, whole-scan accumulation,
late source captures, eviction/drop counters, delete/recreate and reset.
Scene reset is enabled only for the runner's isolated new instance. Neither recipe
establishes the future renderer's visual/scale acceptance.

Discovery uses the public agentio directory query directly on the selected host's
raw `__agentio_resolve` topic. Only a successful negative answer means unsupported;
transport errors are not recast as feature absence. Signed topic ownership, lease,
exchange/type and exact path are checked before opening the data route. Records
owned by another host are rejected rather than silently following a referral.
The reserved control topic must bypass application-topic slash normalization.

Request tokens do not expose authenticated caller identity. Writer
tokens must be unguessable bearer capabilities under existing transport ACLs, with
host-wide quotas; per-writer limits alone do not establish per-peer isolation.

## Bounded list/status requests

`DataListQuery` carries a canonical namespace, limit 1–128, and optional nonempty
cursor of at most 128 UTF-8 bytes. The default limit is 64. `NamespaceFilter` uses
the same canonical namespace checks. `DataStatusQuery` carries an optional nonzero
generation and optional view identity with both ID and incarnation nonzero. Unknown
and duplicate JSON fields fail. Live listing merges sorted legacy and typed identities,
bounding encoded bytes as well as item count. Opaque tokens bind namespace, limit and
listing revision; mutation, timeout or oldest-token eviction yields `stale_cursor`.
Identical page retries reuse the same cursor without consuming another slot. A single
legacy summary too large for the metadata envelope returns `budget`, never a truncated ID.

## Identity and lifecycle facade

`gearbox_api::draw_data::DrawingStore<T>` owns the real legacy `DrawStore`, bounded
dataset keys and pending publication permits, and immutable `Arc<T>` payloads.
It reserves one owner/kind per exact `(namespace, id)`, checks base incarnation and
generation at commit, and invalidates pending permits on delete/namespace clear.
Deleting and recreating a key changes its incarnation. Permits carry a random store
identity so a token from a reconstructed or different store cannot publish data.

Legacy UPSERT/completed APPEND/EXTEND retain their existing cross-legacy-kind behavior.
Only Legacy/Dataset or different dataset-kind ownership conflicts require deletion.
Pending legacy uploads also reserve their keys. Legacy key bounds are unchanged;
new `DataKey` bounds must not truncate or reject old identities at the facade boundary.
Native hosts use this facade with real charged `Generation` payloads. Read guards expose
legacy queries without mutable dereferencing; mutations pass through shared ownership
and deletion barriers. `FakeScene::clone` shares its drawing backend, rather than copying
legacy drawings; its other scene fields retain their ordinary value-clone behavior.

## Bounded reply contracts

`DrawSummaryPage` contains schema revision, a listing revision, up to 128 strictly
sorted unique summaries, and an optional bounded cursor. The complete encoded JSON
must fit 12 KiB independently of the item limit. Legacy summaries preserve the full
original namespace/id; oversized summaries fail explicitly, never truncate identity.
Dataset summaries carry kind, incarnation/generation, scan/page/source/valid counts
and payload bytes. Listing carries no point arrays or full legacy entity payloads.

`DatasetStatus` separates retained/requested dataset revisions, transfer progress,
failures, current resource usage, cumulative counters and view-scoped visibility.
Visibility is one of unavailable, pending, visible, superseded, view-gone,
history-expired or failed. A visible result identifies the exact requested dataset,
view incarnation, completed nonzero frame, and drawn/culled/empty disposition.
Retention is not visibility. A superseding generation must be newer in the same
dataset incarnation and actually retained; it cannot acknowledge the old request.

Status/list JSON uses a bounded writer that stops at 12 KiB while encoding, including
escaped strings. Resource categories may overlap because allocations are shared:
CPU bytes are the unique total, not the sum of every descriptive category. Actual
resource accounting and bounded listing are integrated. Status transfer/failure belongs
to the requested key, while resource and cumulative counters are host-wide. Shared
pages are not falsely charged once per key. Pinned-retired bytes exclude current
retained, incomplete and staged payloads. Native compact clouds now publish per-view
frame receipts; API-only hosts and unsupported kinds still report unavailable.
No successful retention reply alone claims a rendered frame.

### Exact native cloud visibility

The Rust client exposes `wait_visible(key, revision, view, timeout)` and
`wait_visible_with_deadline(key, revision, view, deadline)`. They return
`RenderCompletion` only for that exact dataset incarnation/generation. Passing `None`
resolves the primary interactive view on the first status request, then freezes its
ID and epoch for all subsequent polls. Pass an explicit `ViewIdentity` when the target
must already be fixed before that first request.

```rust,ignore
let mut data = client.draw_data()?;
let completed = data.wait_visible(
    &key,
    DatasetRevision { incarnation: retained.incarnation, generation: retained.stored_generation },
    None,
    std::time::Duration::from_secs(5),
)?;
```

`WaitVisibleError::Terminal` preserves structured superseded, view-gone,
history-expired, failed or unavailable state. `WaitVisibleError::Request` covers
transport, protocol and timeout failures. The wait sends only STATUS requests; a
timeout never cancels a retained dataset. Earlier recorded successes remain queryable
in bounded history, and never inherit a later generation's completion.

The bounded Rust pipeline also exposes
`wait_visible(key, revision, view, deadline, cancellation)`. The optional view is
`(view_id, view_epoch)`; an omitted view freezes on the first status reply. The receipt
holds `target`, `frame`, `disposition`, `source_samples` and `selected_items`. Drawn,
empty and culled dispositions all count as completion; none promises that every source
sample is a visible screen pixel.

Waiting reserves 40 KiB of scratch against the shared queue budget and admits at most
one STATUS per waiter. Insufficient scratch returns `full` before publication. Operation
queue pressure waits within the same absolute deadline. Each request also retains the
pipeline's configured request-timeout cap. No writer lease or mutation is created.
A cancellation token interrupts admission, in-flight waiting and pending-receipt
polling; deadline/cancellation/shutdown attempts to remove queued STATUS. An already
in-flight read can finish within its existing request bound. Scratch and request
charges are released separately as their ownership ends.

Header/receipt identity mismatches are protocol failures, not permission to follow a newer
generation or replacement view. Timeout or cancellation never clears retained data.

For compact clouds, a receipt requires every selected page's actual draw command,
the target view's finished render pass and completion of the submitted GPU work.
Empty clouds require the completed view pass too. Source and selected counts describe
the frontier, not a guarantee that every sample passed clipping, depth or occlusion.
The compact renderer reports drawn, empty and wholly box-clipped frontiers.

Level of detail runs on the GPU. Each full page uploads its hierarchy nodes (80 bytes each)
and refinement order (4 bytes per point) once. Every frame, each view runs a compute pass
before its main pass:

1. Every visible node adds the points it would draw at each error threshold to a
   histogram.
2. One invocation picks the finest threshold, no coarser than 1.5 px, that keeps the view
   within its visible-point budget.
3. Each page lists the refinement-order range every node draws at that threshold, and
   draws it indirectly.
4. Clip-accepted points are counted on the GPU.

The selection is a cut through each page's hierarchy: no point is drawn twice, and no part
of a visible page is left out. Each node keeps the level it drew last frame, and a level it
refined past stays refined until its error falls below 85% of the threshold. A view keeps
last frame's threshold while only one bin (2^(1/4)) finer would fit its budget; the budget
itself always holds. A page keeps its slot, and so its node states, while it stays in view.
Coarse pages draw every coarse record. A frame's receipt reads back the GPU counts after the
submitted work completes, so `selected_items` stays exact. A pick on a view with full pages
asks its next LOD pass for a copy of the node tables and resolves against exactly the points
that frame listed, usually a few frames after the click.

The native roster excludes sensor cameras, supports window-backed interactive views
and Gearbox's embedded ChaseCamera viewport, and preserves identity during temporary
inactivity. Camera/target replacement invalidates old callbacks. At most 16 views and
two submitted proof batches are admitted; proof metadata and retained references are
charged and released.
Each registered view selects a complete prepared generation independently. A ready
camera does not promote the replacement for other cameras, which retain their previous
selection. Receipt entries identify the selected generation of that view, not a global
displayed-generation map. Main/render ownership retains the last extracted candidates
alongside selected generations until they are no longer needed.

The native multiview harness verifies delayed replacement, exact history/supersession,
pause/resume, view replacement and clear/recreated datasets using the production queue
and GPU-completed frames. Its opt-in second image camera and one-page readiness injector
are excluded from ordinary builds. Its quota extension holds drawing GPU-ledger
reservations and verifies allocation and staging denial separately, fresh old-generation
frames in both cameras, and automatic recovery when the reservation is released.
This is not physical VRAM exhaustion, device-loss recovery, multiple LOD cuts or
integrated pressure/scale acceptance; those gates remain open.

Cloud GPU allocation and staging denial report `failure.phase = rendering`,
`code = budget`, with a reason identifying allocation or staging. CPU staging/metadata
denial is reported separately. Exceeding device buffer/binding limits reports
`unsupported`. Status retains the stored revision; an unseen requested revision reports
failed visibility while historical successful receipts remain successful. Preparation
retries without another submission, preserving previous complete visible generations.
Buffer failures clear only when all nonempty pages finish preparation. Required frost
targets have separate admission ownership: successful buffer preparation cannot clear
a target-allocation failure. A normal per-frame upload allowance delay does not create
an error or clear an existing admission failure.

### World frost target ownership

World-space drawing frost targets share the drawing GPU cap with records, uniforms,
staging and deferred retirement. STATUS `resources.gpu_bytes` includes actual texture
width × height × format bytes while drawings own/use the target. This is owned resource
accounting, not measurement of physical driver memory. Existing borrowed transmission
targets are shared without double charging; host panel/UI backdrop resources are separate.

A required target that cannot fit reports Rendering/BUDGET with a `drawing frost`
reason; unsupported dimensions/formats report UNSUPPORTED. The affected view keeps
its last complete generation and retries automatically. It cannot acknowledge a new
frosted generation merely because its point buffers are ready. Target resize, view
removal, blur disable and clear release ownership after submitted GPU work completes.
Opaque blur-zero datasets do not need these targets.

`oslo make test-draw-auxiliary` verifies actual two-view texture dimensions, quota
rejection/recovery, secondary resize, uniform-only blur edits and final zero GPU usage.
Its twelve images include old-generation continuity and clear positive/negative controls.
`test-draw-auxiliary-fixtures` covers the image oracle and extent/format byte calculation.
Neither gate establishes full CPU cache accounting, device-loss recovery or later LOD.

### Integrated million-point example

`oslo make preview-draw-cloud-scale` builds the normal viewer and opens one million
source-colored samples in the owned drawing preview. The `draw_cloud_scale` example
uses the public typed writer, 16 bounded pages, stable u64 source IDs and exact
completed-frame waits. It clears only its demo namespaces and the earlier cloud-basics
scene. The synthetic heightfield is a point cloud, not a reconstructed elevation map.

`oslo make probe-draw-cloud-scale` enables the opt-in renderer observer and verifies
actual live Buffer sizes, upload allowance, uniform-only presentation edits and exact
clear/reload ownership. The default record buffers total 16,000,000 bytes for one
million source-colored points; style/placement uniforms are separately measured.
The observer's charged GPU usage includes in-flight staging/retirement; it does not
measure total physical driver VRAM. Normal builds exclude this observer.

`oslo make capture-draw-data-views --args='OUTPUT cloud-scale'` captures guarded
near/grazing/overview views; omitting the scene keeps the existing cloud-basics behavior.
These allocation/correctness gates do not establish screen-error LOD, residency,
controlled performance or release acceptance. Auxiliary target accounting has its own
native gate above; complete CPU cache/scratch ownership remains under implementation.

### Native cloud sensor exclusion

`oslo make test-draw-sensors --args=OUTPUT` launches an owned isolated headless viewer
and checks both optimized and full robot-camera outputs. In addition to legacy
drawings, it uploads a typed live-link cloud through `draw_cloud_sensors`, tests opaque
and translucent/frosted presentation revisions, and clears its own namespace. Exact
viewer Drawn receipts and captured viewer pixels form positive controls; real sensor
images must remain unchanged within the fixture's declared image tolerance.

`oslo make test-draw-cloud-sensor-fixtures` checks source payloads, launcher syntax
and the image oracle, including rejection of absent viewer draws, camera contamination
and failed clear. Neither fixture-only checks nor a clean sensor image without a
viewer positive control establish native exclusion. The native runner leaves the
user's drawing preview untouched and stops only its own headless instance.

### Native cloud frame transitions

`oslo make test-draw-cloud-frames` checks world, odom, live robot and captured-snapshot
clouds in an owned isolated native viewer. Robot motion, restyling, local-pose delta
and explicit capture replacement must preserve unchanged point-record uploads.
Independent world-position references verify the resulting snapshot positions.
The fixture supplies synthetic external capture metadata; it does not acquire LiDAR
or establish native acquisition-time transforms. The user preview is left untouched.

`oslo make test-draw-cloud-frame-fixtures` checks exact source payloads, launcher syntax
and image-oracle rejection cases. Opt-in native counters distinguish position,
attribute, style and placement uploads; normal builds exclude them. Frame metadata
reuses unchanged attributes while recomputing clipping and placement. Source-coordinate
colors still rebuild values when their source pose changes. Changed acquisition origins
revalidate offset overflow for age attributes.

### Compact legacy POINTS compatibility

The native viewer can draw eligible legacy POINTS without converting their public
identity or payload to typed datasets. PLAIN, HAIRLINE, ORB, POINT_GLOW and SPLAT
single-sprite styles use 12-byte position and 12-byte color/radius/phase records,
plus a 128-byte entity uniform. Their original stroke shader remains the
color, size, depth and visual-style authority. Position/attribute owners are reused
when unchanged; color/radius edits do not require position uploads.

Decorated styles, labels, terrain draping and unknown flags use the existing generic
path. Caller colors, palettes, signed metre/pixel radii and supported shape/on-top
flags keep their legacy semantics. Blur remains the existing 0..1 background-frost
control and does not change opaque SPLAT into a transmitting material. Legacy store,
frame root and lifecycle remain authoritative; no typed dataset or visibility receipt
is fabricated. Both compact renderers consume one shared per-frame upload allowance.

`oslo make test-draw-legacy-points` checks record ownership and independent generic
stroke-input parity. `test-draw-legacy-points-native` builds the ordinary viewer and
compares twenty-one isolated compact/generic cases, combined points and clear. Its
neutral checker background separates glyph differences from animated vegetation.
The additional case overlaps translucent clouds whose entity origins reverse their
actual point-depth order. Sorting uses the transformed source-position bounds.
`test-draw-legacy-lifecycle` checks public retained mutations, explicit generic fallback,
cleanup/TTL and actual robot-frame motion in main 4x and secondary 1x views, using
opt-in actual buffer/upload observations. The sensor fixture independently checks
compact legacy viewer-positive controls and optimized/full robot-camera exclusion.
Results and remaining gates are tracked in the
[integration checkpoint](../plans/research/phase4-legacy-integration-checkpoint.md).

### Source-frame cloud clipping boxes

Cloud presentation accepts optional `clip: {min_m: [x,y,z], max_m: [x,y,z]}`.
Bounds use the scan's declared source frame, after the scan/page origins and
`frame.pose`, before world/live/snapshot placement. They therefore move with an
externally placed frame. Bounds must be finite with strictly positive finite extents.
Omitting `clip` (or setting it to null) leaves samples unclipped and preserves the
previous default serialization. This is independent of octree display ROI settings.

The box selects sample centers inclusively; it does not slice the edge of each
billboard disc. Measurements and source IDs remain unchanged. Clip-only presentation
patches publish a new generation while sharing position and attribute buffers.
Page-relative bounds and rotation are checked against one-millimetre local GPU
precision; an unrepresentable boundary/rotation reports a preparation failure rather
than silently moving the box. The CPU count and shader use the same fused operation
order, tested against actual GPU results.

Completed receipts preserve source_samples and report the box-selected valid samples
as selected_items. Nonempty valid data wholly outside the box is `culled`, not `empty`;
that result still requires its actual page draws, view pass and GPU completion.
Clipping planes, interactive ROI handles and picking parity remain Phase 8 work.

### Continuous ramps and indexed cloud palettes

TURBO, VIRIDIS, RDYLGN, RAINBOW and GREY use the normalized caller range; a constant
range selects the midpoint. COSTMAP, MAP, RAW and OKABE keep the existing API's indexed
semantics instead: apply declared attribute normalization, round ties away from zero,
and look up the raw index without interpolation or range remapping. COSTMAP/MAP treat
negative rounded values as unknown 255 and clamp large values to 255. RAW clamps to
0..255; OKABE wraps by Euclidean modulo eight. An unnormalized u64 retains its exact
integer index, even beyond f64's integer precision. Finite f64 indices use their own
precision rather than a prior f32 conversion. Original retained values are unchanged.

Positions are 12-byte f32 local offsets; the source row of each point stays on the
CPU. Attribute words hold only what the selected color mode reads: none for constant
color, one RGBA word, one value word, one category word, or two acquisition-offset
words for age, plus one f32 word when a radius attribute is selected. Value, category
and age pages add one validity bit per point; an invalid RGBA sample is stored as zero.
A value word keeps the ramp position in 8.12 fixed point (4,096 steps between
neighbouring ramp entries), the raw palette byte, the Okabe slot and a below-zero flag.
A source- or scalar-colored point therefore costs 16 bytes on the GPU. Switching
these maps changes only style uniforms and reuses geometry and attribute buffers.
Axis/range colors use the same source-frame scalar semantics.

Age normalization uses a per-page integer nanosecond anchor and finite scale rather
than an f32 timestamp or f32 age range. Source timestamps are subtracted as integers;
simulation timestamps use compensated CPU differences before subtracting the integer
sample offset. The GPU subtracts acquisition offsets before conversion to f32, retaining
narrow color windows at large clock origins. Sub-nanosecond windows use exact integer
ordering around their possible in-window sample; equal ranges still select the midpoint.
Indexed age maps use signed whole seconds, a nanosecond threshold and modulo-eight
phase, preserving future-age and half-tie semantics without rounding a large f32 value.
Changing age clock, range or palette does not reconstruct point buffers. Age uniforms
cost 48 bytes within each 240-byte page uniform; the style uniform is 5,184 bytes.
All are charged separately from the per-point records.

Source-age presentation references the maximum retained capture timestamp in the same
named source clock and scene epoch. Unrelated clock names never advance one another.
For source-aged accumulation, the authoritative committed retention anchor also
participates, preserving its monotonic watermark after empty replacement or late input.
Simulation-aged accumulation has no source-clock history: removing its newest source
capture can lower that domain's color reference. Its simulation retention anchor does
not participate in source colors. Color changes neither alter retention nor create a
clock history. These references derive from the immutable generation, independent of
which intermediate generations a renderer displayed. Delete/recreate and explicit
stream-policy replacement retain their existing anchor-reset semantics.

Null scalar data uses the explicit missing RGBA, never zero's palette entry. A
zero-alpha palette entry is a cutout, not an invented color. Samples with unit effective
alpha use the depth-writing opaque pass; fractional samples use depth-tested premultiplied
transmission without depth writes. Mixed pages participate in both passes, filtering
fragments by their actual sample alpha without duplicating or repacking point records.
Unused missing-data alpha does not force valid opaque measurements into transmission.
Pass routing is generation-local; an opacity/palette change still shares prepared pages.

Opaque disc edges use alpha-to-coverage with multisampling. With one sample, only fully
covered opaque interiors write depth, while the transmission pass blends fractional
coverage edges. Those edges do not apply background frost: only actual fractional
sample opacity transmits a frosted background. Exact visibility receipts require every
required pass of each page as well as the completed view/frame. Selected counts remain
geometric/validity/clip counts, not counts of pixels surviving alpha or depth tests.

Glow preserves the API's finite nonnegative value; there is no input clamp at four.
Final cloud radiance is bounded to 65,504, the finite half-float render-target limit,
to avoid nonfinite output for extreme strengths. Color choice does not force frost,
glow or a shared accent tint onto measured data. Transparent samples retain the existing
page-sorted blending model; this is not order-independent per-point transparency.

## Native LiDAR capture extension

`NativeLidarCapture` is optional JSON under the existing `capture.v1` LiDAR prop:
revision 1, machine, link, `right_handed_z_up_metres` convention, and the complete
`CaptureTransform`. That transform carries source/target frames, capture-time f64
pose, anchored datum and capture stamp, including exact u64 sample/scene epoch and
optional named source-clock nanoseconds. Maximum capture JSON is 2 KiB; capture
props are bounded to 4 KiB and 32 entries. Source frame must equal `machine/link`.

`LidarScan::native_capture()` distinguishes absent metadata from malformed metadata.
The capture time equals the existing binary simulation timestamp exactly, sample
low 32 bits equal the unchanged header, and the `name` prop matches the link.
`set_native_capture()` validates shape/bounds and restores old props if the resulting
packet exceeds 16 KiB. `lidar_chunk_capacity()` calculates capacity from actual packed
bytes; ordinary legacy props retain the existing 900-pulse maximum.

The fixed `gearbox.lidar_scan.v1` header, section layout and hash are unchanged.
Serde JSON enables `float_roundtrip`; a 4,096-value regression verifies exact f64
timestamps instead of weakening equality to an epsilon. Sensor dispatch does not
yet emit this extension: Phase 7 must snapshot actual dispatch-time pose and datum.
It must not use the robot's later live pose when captured metadata is absent/invalid.

## Frozen bytes and cross-client checks

`tests/fixtures/draw_data/wire-v1.json` under `crates/gearbox-api` contains 37 packets
and three page fixtures, their human-readable fields and CRCs; `SHA256SUMS` freezes
the file. Coverage includes all 16 operations, cloud/octree begin metadata, all four
reply stages and 13 errors, exact u64 extrema, raw JSON ordering/spacing, unchanged
legacy drawing/LiDAR bytes, optional capture props, all eight scalar encodings,
normalized integers, validity masks, and known/unknown mixed-depth occupancy leaves.

Request header: `<4I9Q6I` (112 bytes). Reply header: `<4I8Q4I` (96 bytes).
Little-endian fixed fields are followed by `(offset, byte_length)` pairs in section
order. Offsets are relative to the concatenated payload; even empty sections use
the current cursor. There is no padding or magic prefix. The canonical type name is
hashed with FNV-1a u64. Reordered, overlapping, gapped or trailing bytes are rejected.

The CLI's registered data JSON descriptors use raw `key_json`/`metadata_json` strings
and `payload_hex` with a required `0x` prefix. Raw strings preserve exact section
bytes across conversion; typed metadata is checked before transport. Numeric header
values remain unsigned integers, not floating point or string props. For example:

```sh
gearbox --instance INSTANCE --json api call /gearbox/draw/data/capabilities '{}'
nix develop --offline --command oslo make test-draw-data-wire
```

`update-draw-data-wire` is a separate explicit fixture regeneration recipe. Ordinary
tests never rewrite expected bytes. The native `test-draw-live` checks the actual CLI
capability route as well as Rust negotiation and the legacy drawing lifecycle.

## Field queries

`FIELD_QUERY` (op 17) runs on the status topic against a retained volume. The request
metadata is a `FieldQuery`:
- `points`: 1..64 finite volume-frame points;
- `channel`: `value` or `weight`;
- `interpolation`: `nearest` or `trilinear`;
- `gradient`: a bool;
- `generation`: optional.

The reply is a `FieldAnswer`: the revision read, the field, `spacing_m` and one
`FieldSample { value, support, gradient }` per point.
- `support` is `valid`, `unknown` or `unallocated`; a value is present only when
  support is valid.
- Trilinear needs all eight surrounding voxel centres valid.
- The gradient is a central difference one voxel apart.
- A query naming an older generation gets `SUPERSEDED`.

## Ground-relative origins

`DataFrame.ground_radius_m` (optional, 0..200, live `world` frames only) makes the
origin's z metres above the highest terrain within that radius of the origin. A rigid
dataset placed on a slope then never sinks into it. When the field is absent, the origin
is absolute.

## Not implemented

- Eye-Dome Lighting (Phase 13, deferred).
- LOD error and upload work in `STATUS`; the `draw-probes` recorder tracks them per frame.
