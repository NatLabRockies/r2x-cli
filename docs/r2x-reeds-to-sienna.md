# ReEDS to Sienna Pipeline

## Required Plugins

```bash
r2x install r2x-reeds
r2x install r2x-sienna
r2x install r2x-reeds-to-sienna
```

> **Note:** A `pcm_defaults.json` file is optional to cover properties not set in the original ReEDS run. You can create one or use the [default config](https://github.com/NatLabRockies/r2x-reeds/blob/main/src/r2x_reeds/config/pcm_defaults.json).

## Pipeline YAML

```yaml
variables:
  model_name: <reeds_run_name>
  reeds_run: <reeds_run_path>
  output_dir: <output_path>
  solve_year: 2035
  weather_year: 2012

pipelines:
  r2s:
    - r2x-reeds.reeds-parser
    - r2x-reeds.break-gens
    # - r2x-reeds.add-pcm-defaults
    - r2x-reeds-to-sienna.reeds-to-sienna
    - r2x-sienna.sienna-exporter

config:
  r2x-reeds.reeds-parser:
    weather_year: ${weather_year}
    solve_year: ${solve_year}
    path: ${reeds_run}

  r2x-reeds.break-gens:
    drop_capacity_threshold: 5

  # r2x-reeds.add-pcm-defaults:
  #   pcm_defaults_fpath: <path_to_pcm_defaults.json>
  #   pcm_defaults_override: true

  r2x-reeds-to-sienna.reeds-to-sienna:
    solve_year: ${solve_year}

  r2x-sienna.sienna-exporter:
    model_year: ${solve_year}
    system_name: ${model_name}
    system_base_power: 100.0
    skip_validation: true
    output_path: ${output_dir}/${model_name}.json

output_folder: ${output_dir}
```

## Running

```bash
r2x run reeds-to-sienna-pipeline.yaml r2s
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

### `r2x-reeds.add-pcm-defaults` (optional)

- `pcm_defaults_fpath`: path to a PCM defaults JSON file.
- `pcm_defaults_override`: when `true`, overrides existing properties.

### `r2x-reeds-to-sienna.reeds-to-sienna`

- `solve_year`: required; year used for scenario mapping.

### `r2x-sienna.sienna-exporter`

- `output_path`: required; output Sienna JSON file path.
- `model_year`: optional; model year tag.
- `system_name`: optional system name in the output.
- `scenario`: optional scenario label; default is `base`.
- `system_base_power`: optional base MVA; default `100.0`.
- `skip_validation`: optional; default `false`.
- `models`: optional tuple of model module paths.

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
r2x run reeds-to-sienna-pipeline.yaml --list

# print resolved config
r2x run reeds-to-sienna-pipeline.yaml --print r2s

# preview without executing
r2x run reeds-to-sienna-pipeline.yaml r2s --dry-run
```
