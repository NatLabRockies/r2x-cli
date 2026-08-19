# PLEXOS to Sienna Pipeline

## Required Plugins

```bash
r2x install r2x-plexos
r2x install r2x-sienna
r2x install r2x-plexos-to-sienna
```

## Pipeline YAML

```yaml
variables:
  run_name: <plexos_run_name>
  model_name: <plexos_model_name>
  plexos_dir: <plexos_run_path>
  output_dir: <output_path>
  solve_year: 2035
  weather_year: 2012

pipelines:
  p2s:
    - r2x-plexos.plexos-parser
    - r2x-plexos-to-sienna.plexos-to-sienna
    - r2x-sienna.sienna-exporter

config:
  r2x-plexos.plexos-parser:
    model_name: ${model_name}
    weather_year: ${weather_year}
    solve_year: ${solve_year}
    fpath: ${plexos_dir}

  r2x-plexos-to-sienna.plexos-to-sienna:
    solve_year: ${solve_year}

  r2x-sienna.sienna-exporter:
    model_year: ${solve_year}
    system_name: ${run_name}
    system_base_power: 100.0
    skip_validation: true
    output_path: ${output_dir}/${run_name}.json

output_folder: ${output_dir}
```

## Running

```bash
r2x run plexos-to-sienna-pipeline.yaml p2s
```

## Entry Details

### `r2x-plexos.plexos-parser`

- `fpath`: path to a PLEXOS XML file or a directory containing XML files.
- `model_name`: model object name to select from the XML database.
- `horizon_year`: optional horizon filter for parsing behavior.
- `solve_year`: optional; supports `int` or `list[int]`.
- `weather_year`: optional; used to label time-series data.
- `timeseries_dir`: optional existing directory with time-series files.

### `r2x-plexos-to-sienna.plexos-to-sienna`

- `solve_year`: required; year used for scenario mapping.

### `r2x-sienna.sienna-exporter`

- `output_path`: required; output Sienna JSON file path.
- `model_year`: optional; model year tag.
- `system_name`: optional system name in the output.
- `scenario`: optional scenario label; default is `base`.
- `system_base_power`: optional base MVA; default `100.0`.
- `skip_validation`: optional; default `false`.
- `models`: optional tuple of model module paths.

## Validation Commands

```bash
# list available pipelines
r2x run plexos-to-sienna-pipeline.yaml --list

# print resolved config
r2x run plexos-to-sienna-pipeline.yaml --print p2s

# preview without executing
r2x run plexos-to-sienna-pipeline.yaml p2s --dry-run
```
