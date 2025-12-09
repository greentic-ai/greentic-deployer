# Flow Authoring (YGTC)

YGTC flows are YAML documents validated by `schemas/ygtc.flow.schema.json`. A flow is an object with required `id` and `type`, optional `title`, `description`, `start`, `parameters`, and a `nodes` map. `parameters` is arbitrary JSON passed through unchanged.

## Nodes and routing

- Node ids must match `^[a-zA-Z_][\w-]*$`.
- Each node map must contain exactly one component entry keyed as `<namespace>.<adapter>.<operation>` or a builtin (`questions` / `template`).
- Component payloads are opaque JSON blocks left as authored.
- Optional `routing` is an array of `{ to?: <node-id>, out?: <bool> }`. `out: true` marks a terminal edge; `to: "out"` is allowed but the boolean is the canonical signal.

Example:

```yaml
id: weather
type: events
title: Weather bot
nodes:
  in:
    channel.slack.receive: {}
    routing:
      - to: detect_intent
  detect_intent:
    nlu.router.detect:
      model: "gpt-4o"
    routing:
      - to: ask_location
  ask_location:
    questions:
      prompt: "Which city?"
    routing:
      - to: format_reply
  format_reply:
    templating.handlebars:
      text: "{{weather}}"
    routing:
      - out: true
```

`start` is optional; the loader uses the provided value when set, otherwise defaults to node `in` when present.

## Loading and validation

- `loader::load_ygtc_from_str` / `load_ygtc_from_str_with_source` parse YAML, convert to JSON, validate against the schema, and materialize a `FlowDoc`.
- Schema paths are normalized via `path_safety::normalize_under_root`.
- The loader extracts the single component key and payload, erroring on zero or multiple components (`FlowError::NodeComponentShape`) or malformed keys (`FlowError::BadComponentKey`).
- Routing JSON deserializes into `Route`; any `route.to` other than literal `"out"` must name an existing node (`FlowError::MissingNode`).
- When `start` is absent and node `in` exists, it is auto-selected as the entry point.
- `resolve::resolve_parameters` can pre-resolve `parameters.*` strings; everything else stays opaque JSON.

### Authoring model

- `FlowDoc { id, flow_type, title, description, start, parameters, nodes }`.
- `Node { component, payload, routing: Vec<Route>, raw }` with `raw` cleared after component extraction.
- `Route { to: Option<String>, out: Option<bool> }`.

## IR conversion

- `to_ir(flow: FlowDoc) -> FlowIR` (in `src/lib.rs`) maps the authoring model into IR.
- `RouteIR.out` defaults to `false` when unset; routing stays per-node.
- `classify_node_type` treats strings with three or more dot segments as adapters (`namespace.adapter.operation`); anything else is a builtin.
- `FlowIR`/`NodeIR` keep `component` as a plain string and `payload_expr` as untyped JSON; there is no WIT/interface awareness.

## Bundles and linting

- `FlowBundle` captures canonicalized flow JSON plus a BLAKE3 hash and a list of `NodeRef { node_id, component: ComponentPin, schema_id }`; the entry point prefers `start`, else `in`, else the first node.
- `ComponentPin.version_req` defaults to `"*"`, `schema_id` is `None`.
- Optional adapter linting (`lint::adapter_resolvable`) uses `classify_node_type` and an adapter catalog to flag missing adapters; routing semantics remain data-driven.
- Serialization is plain `serde_json`; no CBOR is used.

## Fixtures and special nodes

- YAML fixtures (e.g. `fixtures/weather_bot.ygtc`, `tests/data/flow_ok.ygtc`) demonstrate the schema shape.
- `questions` and `template` are builtin node types used in the config-flow fixture (`tests/data/config_flow.ygtc`).
- Branching is expressed with multiple routes; terminating a flow uses `out: true`. Reply-to-origin or other patterns are left to runtimes outside this crate.
