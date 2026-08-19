# Sienna to PLEXOS Pipeline

## Required Plugins

```bash
r2x install r2x-sienna
r2x install r2x-plexos
r2x install r2x-sienna-to-plexos
```

## Pipeline YAML

```yaml
variables:
  system_name: <sienna_system_name>
  sienna_system: <path_to>/${system_name}.json
  output_dir: <output_path>
  plexos_template: PLEXOS10.0  # supports 9.0, 10.0, 11.0, 12.0
  model_year: 2023
  weather_year: 2012

pipelines:
  s2p:
    - r2x-sienna.sienna-parser
    - r2x-sienna-to-plexos.sienna-to-plexos
    - r2x-plexos.plexos-exporter

config:
  r2x-sienna.sienna-parser:
    model_name: ${system_name}
    weather_year: ${weather_year}
    solve_year: ${model_year}
    json_path: ${sienna_system}

  r2x-sienna-to-plexos.sienna-to-plexos:
    solve_year: ${model_year}

  r2x-plexos.plexos-exporter:
    model_name: ${system_name}
    weather_year: ${weather_year}
    horizon_year: ${model_year}
    template: ${plexos_template}
    output_path: ${output_dir}

output_folder: ${output_dir}
```

## Running

```bash
r2x run sienna-to-plexos-pipeline.yaml s2p
```

## Entry Details

### `r2x-sienna.sienna-parser`

- `json_path`: required path to the Sienna system JSON file.
- `model_name`: optional system name override.
- `model_year` / `solve_year`: optional; supports `int` or `list[int]`.
- `weather_year`: optional; used to label time-series data.
- `scenario`: optional scenario label; default is `base`.
- `system_base_power`: optional base MVA; default `100.0`.
- `skip_validation`: optional; default `false`.
- `models`: optional tuple of model module paths.

### `r2x-sienna-to-plexos.sienna-to-plexos`

- `solve_year`: required; year used for scenario mapping.

### `r2x-plexos.plexos-exporter`

- `output_path`: output directory for XML/CSV export artifacts.
- `model_name`: model name in the exported PLEXOS data.
- `horizon_year`: recommended; required when building simulation config from scratch.
- `template`: optional template selector. Can be a known key (e.g. `PLEXOS10.0`) or an XML template file path.
- `weather_year`: optional; used to label time-series data.
- `simulation_config`: optional advanced simulation config object.

## Validation Commands

```bash
# list available pipelines
r2x run sienna-to-plexos-pipeline.yaml --list

# print resolved config
r2x run sienna-to-plexos-pipeline.yaml --print s2p

# preview without executing
r2x run sienna-to-plexos-pipeline.yaml s2p --dry-run
```
