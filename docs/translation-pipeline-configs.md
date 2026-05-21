# Translation Pipeline YAML Configurations

This guide documents how to configure `r2x-cli` pipeline YAML files for the main translation flows that involve:

- `r2x-reeds`
- `r2x-sienna`
- `r2x-plexos`

It focuses on:

- the YAML structure expected by `r2x-cli`
- four common translation configurations
- the config entries used by each pipeline step

## Pipeline YAML Structure

A pipeline file has four top-level sections:

```yaml
variables:
  # reusable values

pipelines:
  # named plugin step lists

config:
  # per-plugin configuration

output_folder: ${some_variable}
```

`r2x-cli` supports variable substitution in strings using both `${var}` and `$(var)`.

## Naming Convention For Plugin Steps

In `pipelines`, each step uses the plugin identifier:

`<package-name>.<plugin-name>`

Examples:

- `r2x-reeds.reeds-parser`
- `r2x-sienna.sienna-parser`
- `r2x-sienna.sienna-exporter`
- `r2x-plexos.plexos-parser`
- `r2x-plexos.plexos-exporter`

The same identifier must be used under `config`.

## Configuration 1: ReEDS -> Sienna

```yaml
variables:
  reeds_run: /data/reeds/run
  solve_year: 2032
  weather_year: 2012
  sienna_output: output/reeds_to_sienna/system.json

pipelines:
  reeds-to-sienna:
    - r2x-reeds.reeds-parser
    - r2x-sienna.sienna-exporter

config:
  r2x-reeds.reeds-parser:
    path: ${reeds_run}
    solve_year: ${solve_year}
    weather_year: ${weather_year}
    case_name: ReEDS-Scenario
    scenario: base

  r2x-sienna.sienna-exporter:
    output_path: ${sienna_output}
    scenario: base
    system_base_power: 100.0

output_folder: output/reeds_to_sienna
```

### Entry Details

`r2x-reeds.reeds-parser`

- `path`: ReEDS run folder path used by the parser data store.
- `solve_year`: required; supports `int` or `list[int]`.
- `weather_year`: required; supports `int` or `list[int]`.
- `case_name`: optional ReEDS case label.
- `scenario`: optional scenario label; default is `base`.

`r2x-sienna.sienna-exporter`

- `output_path`: required; output Sienna JSON file path.
- `scenario`: optional metadata scenario; default is `base`.
- `system_base_power`: optional base MVA; default `100.0`.
- `models`: optional tuple of model module paths; default is Sienna models.

## Configuration 2: ReEDS -> PLEXOS

```yaml
variables:
  reeds_run: /data/reeds/run
  solve_year: 2032
  weather_year: 2012
  plexos_output_dir: output/reeds_to_plexos
  plexos_model_name: ReEDS2032

pipelines:
  reeds-to-plexos:
    - r2x-reeds.reeds-parser
    - r2x-plexos.plexos-exporter

config:
  r2x-reeds.reeds-parser:
    path: ${reeds_run}
    solve_year: ${solve_year}
    weather_year: ${weather_year}
    case_name: ReEDS-Scenario

  r2x-plexos.plexos-exporter:
    output_path: ${plexos_output_dir}
    model_name: ${plexos_model_name}
    horizon_year: ${solve_year}
    template: PLEXOS10.0

output_folder: ${plexos_output_dir}
```

### Entry Details

`r2x-plexos.plexos-exporter`

- `output_path`: output directory path used for XML/CSV export artifacts.
- `model_name`: model name in exported PLEXOS data.
- `horizon_year`: recommended; required when exporter must build simulation config from scratch.
- `template`: optional template selector. Can be a known key (for example `PLEXOS10.0`) or an XML template file path.
- `simulation_config`: optional advanced simulation config object.

## Configuration 3: Sienna -> PLEXOS

```yaml
variables:
  sienna_json: /data/sienna/system.json
  model_year: 2032
  plexos_output_dir: output/sienna_to_plexos
  plexos_model_name: Sienna2032

pipelines:
  sienna-to-plexos:
    - r2x-sienna.sienna-parser
    - r2x-plexos.plexos-exporter

config:
  r2x-sienna.sienna-parser:
    json_path: ${sienna_json}
    model_year: ${model_year}
    system_name: SIENNA-CASE
    scenario: base
    skip_validation: false

  r2x-plexos.plexos-exporter:
    output_path: ${plexos_output_dir}
    model_name: ${plexos_model_name}
    horizon_year: ${model_year}
    template: PLEXOS10.0

output_folder: ${plexos_output_dir}
```

### Entry Details

`r2x-sienna.sienna-parser`

- `json_path`: required unless data is piped through stdin.
- `model_year`: optional; supports `int` or `list[int]`.
- `system_name`: optional system name override.
- `scenario`: optional scenario label; default `base`.
- `system_base_power`: optional base MVA; default `100.0`.
- `skip_validation`: optional; default `false`.
- `models`: optional tuple of model module paths.

## Configuration 4: PLEXOS -> Sienna

```yaml
variables:
  plexos_xml: /data/plexos/Base_2024.xml
  plexos_model_name: Base
  horizon_year: 2024
  sienna_output: output/plexos_to_sienna/system.json

pipelines:
  plexos-to-sienna:
    - r2x-plexos.plexos-parser
    - r2x-sienna.sienna-exporter

config:
  r2x-plexos.plexos-parser:
    fpath: ${plexos_xml}
    model_name: ${plexos_model_name}
    horizon_year: ${horizon_year}

  r2x-sienna.sienna-exporter:
    output_path: ${sienna_output}
    scenario: base

output_folder: output/plexos_to_sienna
```

### Entry Details

`r2x-plexos.plexos-parser`

- `fpath`: path to a PLEXOS XML file or a directory containing XML files.
- `model_name`: model object name to select from the XML database.
- `horizon_year`: optional horizon filter for parsing behavior.
- `timeseries_dir`: optional existing directory with time-series files.

## Optional ReEDS Post-Parse Transforms

When your flow starts with ReEDS parsing, you can insert optional transform steps after `r2x-reeds.reeds-parser`, such as:

- `r2x-reeds.break-gens`
- `r2x-reeds.add-pcm-defaults`
- `r2x-reeds.add-emission-cap`
- `r2x-reeds.add-electrolyzer-load`
- `r2x-reeds.add-ccs-credit`
- `r2x-reeds.add-imports`
- `r2x-reeds.add-optimal-siting`

Each transform has its own config block under `config` keyed by the same plugin step name.

## Validation And Inspection Commands

```bash
# validate available pipelines
r2x run pipeline.yaml --list

# print resolved config for one pipeline
r2x run pipeline.yaml --print reeds-to-plexos

# preview execution order and configs without running plugins
r2x run pipeline.yaml reeds-to-plexos --dry-run
```

## Notes

- Keep plugin names in `pipelines` and keys in `config` exactly aligned.
- Prefer variables for paths and years to avoid duplication.
- If a plugin expects a file path, make it explicit in `config` (for example `json_path`, `fpath`, `output_path`).
- For PLEXOS export workflows, set `horizon_year` explicitly unless your upstream metadata already guarantees it.
