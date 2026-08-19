# ReEDS to PLEXOS Pipeline

## Required Plugins

```bash
r2x install r2x-reeds
r2x install r2x-plexos
r2x install r2x-reeds-to-plexos
```

> **Note:** A `pcm_defaults_custom.json` file is optional to cover properties not set in the original ReEDS run. You can create one or use the [default config](https://github.com/NatLabRockies/r2x-reeds/blob/main/src/r2x_reeds/config/pcm_defaults.json).

## Pipeline YAML

```yaml
variables:
  model_name: <reeds_run_name>
  output_dir: <output_path>
  reeds_run: <reeds_run_path>
  plexos_template: PLEXOS12.0  # supports 9.0, 10.0, 11.0, 12.0
  solve_year: 2050
  weather_year: 2012

pipelines:
  r2p:
    - r2x-reeds.reeds-parser
    - r2x-reeds.break-gens
    - r2x-reeds.add-pcm-defaults
    - r2x-reeds-to-plexos.reeds-to-plexos
    - r2x-plexos.plexos-exporter

config:
  r2x-reeds.reeds-parser:
    weather_year: ${weather_year}
    solve_year: ${solve_year}
    path: ${reeds_run}

  r2x-reeds.break-gens:
    drop_capacity_threshold: 5

  r2x-reeds.add-pcm-defaults:
    pcm_defaults_fpath: <path_to_pcm_defaults.json>
    pcm_defaults_override: true

  r2x-reeds-to-plexos.reeds-to-plexos:
    solve_year: ${solve_year}
    hydro_budget_ts: "monthly"

  r2x-plexos.plexos-exporter:
    model_name: ${model_name}
    weather_year: ${weather_year}
    horizon_year: ${solve_year}
    template: ${plexos_template}
    output_path: ${output_dir}

output_folder: ${output_dir}
```

## Running

```bash
r2x run reeds-to-plexos-pipeline.yaml r2p
```

## Entry Details

### `r2x-reeds.reeds-parser`

- `path`: ReEDS run folder used by the parser data store.
- `solve_year`: required; supports `int` or `list[int]`.
- `weather_year`: required; supports `int` or `list[int]`.
- `case_name`: optional ReEDS case label.
- `scenario`: optional scenario label; default is `base`.

### `r2x-reeds.break-gens`

- `drop_capacity_threshold`: capacity (MW) below which generators are dropped.
- `reference_units`: optional path to a PCM defaults JSON to inform unit splitting.
- `break_category`: grouping key for splitting (e.g. `technology`).
- `skip_categories`: list of technology categories to skip during breaking.

### `r2x-reeds.add-pcm-defaults`

- `pcm_defaults_fpath`: path to a PCM defaults JSON file.
- `pcm_defaults_override`: when `true`, overrides existing properties.

### `r2x-reeds-to-plexos.reeds-to-plexos`

- `solve_year`: required; year used for scenario mapping.
- `hydro_budget_ts`: optional hydro budget time-series granularity (`"monthly"`, `"annual"`).

### `r2x-plexos.plexos-exporter`

- `output_path`: output directory for XML/CSV export artifacts.
- `model_name`: model name in the exported PLEXOS data.
- `horizon_year`: recommended; required when building simulation config from scratch.
- `template`: optional template selector. Can be a known key (e.g. `PLEXOS12.0`) or an XML template file path.
- `weather_year`: optional; used to label time-series data.
- `simulation_config`: optional advanced simulation config object.

## Optional ReEDS Post-Parse Transforms

Additional transform steps can be inserted after `r2x-reeds.reeds-parser`:

- `r2x-reeds.break-gens`
- `r2x-reeds.add-pcm-defaults`
- `r2x-reeds.add-emission-cap`
- `r2x-reeds.add-electrolyzer-load`
- `r2x-reeds.add-ccs-credit`
- `r2x-reeds.add-imports`
- `r2x-reeds.add-optimal-siting`

Each transform requires its own `config` block keyed by the same plugin step name.

## Validation Commands

```bash
# list available pipelines
r2x run reeds-to-plexos-pipeline.yaml --list

# print resolved config
r2x run reeds-to-plexos-pipeline.yaml --print r2p

# preview without executing
r2x run reeds-to-plexos-pipeline.yaml r2p --dry-run
```
