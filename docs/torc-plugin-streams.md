# Use r2x plugin streams in Torc

This how-to is for Torc users who want r2x plugins to compose as ordinary Unix commands while Torc owns job expansion, resources, and durable job dependencies.

## Choose the boundary

Use a Unix pipe for work that belongs in **one Torc job**:

```bash
set -o pipefail
r2x run r2x-reeds.reeds-parser \
  --path data/runs/2026_06_18_USA_defaults \
  --solve-year 2050 \
  --weather-year 2012 \
  --scenario A1 |
r2x run r2x-reeds.add-pcm-defaults \
  --pcm-defaults-fpath config/r2x/pcm_defaults.json |
r2x run r2x-reeds-to-plexos.reeds-to-plexos \
  --models '["r2x_reeds.models","r2x_plexos.models"]' |
r2x run r2x-plexos.plexos-exporter \
  --output-path artifacts/A1/2050/plexos \
  --model-name Geothermal_A1 \
  --horizon-year 2050 \
  --weather-year 2012 \
  --template PLEXOS10.0
```

A System-producing plugin writes one JSON document to stdout. Its time-series sidecars are stored in r2x's cache-backed stream location, and the JSON embeds the absolute sidecar path. A following `r2x run` deserializes that System from stdin. Diagnostics go to stderr, and exporters are terminal sinks: they write their configured files and emit no JSON record.

Use `set -o pipefail` so Torc receives a failure from any command in the pipe.

Use `-o` and `-i` for a **durable boundary between Torc jobs**:

```bash
r2x run r2x-reeds.reeds-parser \
  --path data/runs/2026_06_18_USA_defaults \
  --solve-year 2050 \
  --weather-year 2012 \
  --scenario A1 \
  -o artifacts/A1/2050/infrasys.json

r2x run r2x-reeds.add-pcm-defaults \
  -i artifacts/A1/2050/infrasys.json \
  --pcm-defaults-fpath config/r2x/pcm_defaults.json
```

For a System result, `-o artifacts/A1/2050/infrasys.json` creates missing parent directories, replaces an existing JSON entrypoint, and creates both:

```text
artifacts/A1/2050/
├── infrasys.json
└── infrasys_time_series/
```

The durable JSON contains a relative sidecar directory, so consume it with `-i infrasys.json`, not `cat infrasys.json | ...`.

## Streamed generation with Torc file dependencies

Torc's `input_files` and `output_files` name durable filesystem artifacts. They do not represent the live System passed through the pipe. Declare the concrete exporter output that a later job consumes.

```yaml
name: geothermal_reeds_to_plexos

parameters:
  scenario: [A1, A2, A3, B1, B2, B3, C1, C2, C3]
  solve_year: [2050]
  weather_year: [2012]

files:
  - name: plexos_xml_{scenario}_{solve_year}_{weather_year}
    path: artifacts/{scenario}/{solve_year}/plexos/Geothermal_{scenario}_{weather_year}_{solve_year}.xml
    use_parameters: [scenario, solve_year, weather_year]

jobs:
  - name: generate_A_{scenario}_{solve_year}_{weather_year}
    parameters:
      scenario: [A1, A2, A3]
    use_parameters: [solve_year, weather_year]
    command: |
      set -o pipefail
      r2x run r2x-reeds.reeds-parser \
        --path data/runs/2026_06_18_USA_defaults \
        --solve-year {solve_year} \
        --weather-year {weather_year} \
        --case-name geothermal-{scenario} \
        --scenario {scenario} |
      r2x run r2x-reeds-to-plexos.reeds-to-plexos \
        --models '["r2x_reeds.models","r2x_plexos.models"]' |
      r2x run r2x-plexos.plexos-exporter \
        --output-path artifacts/{scenario}/{solve_year}/plexos \
        --model-name Geothermal_{scenario} \
        --horizon-year {solve_year} \
        --weather-year {weather_year} \
        --template PLEXOS10.0
    output_files:
      - plexos_xml_{scenario}_{solve_year}_{weather_year}

  - name: validate_A_{scenario}_{solve_year}_{weather_year}
    parameters:
      scenario: [A1, A2, A3]
    use_parameters: [solve_year, weather_year]
    command: >
      python scripts/geothermal_workflow.py validate
      --scenario {scenario} --variant A
      --solve-year {solve_year} --weather-year {weather_year}
      --output artifacts/{scenario}/{solve_year}
    input_files:
      - plexos_xml_{scenario}_{solve_year}_{weather_year}
```

The `plexos_xml_*` logical file creates the generation-to-validation dependency. B and C use the same shape; they only add their variant-specific modifiers to the live pipe.

## Durable staged generation with Torc file dependencies

When parsing must be scheduled, retried, or inspected separately, make the System entrypoint an explicit Torc artifact.

```yaml
files:
  - name: parsed_system_{scenario}_{solve_year}_{weather_year}
    path: artifacts/{scenario}/{solve_year}/infrasys.json
    use_parameters: [scenario, solve_year, weather_year]

jobs:
  - name: parse_{scenario}_{solve_year}_{weather_year}
    use_parameters: [scenario, solve_year, weather_year]
    command: >
      r2x run r2x-reeds.reeds-parser
      --path data/runs/2026_06_18_USA_defaults
      --solve-year {solve_year}
      --weather-year {weather_year}
      --scenario {scenario}
      -o artifacts/{scenario}/{solve_year}/infrasys.json
    output_files:
      - parsed_system_{scenario}_{solve_year}_{weather_year}

  - name: generate_A_{scenario}_{solve_year}_{weather_year}
    parameters:
      scenario: [A1, A2, A3]
    use_parameters: [solve_year, weather_year]
    command: |
      set -o pipefail
      r2x run r2x-reeds-to-plexos.reeds-to-plexos \
        -i artifacts/{scenario}/{solve_year}/infrasys.json \
        --models '["r2x_reeds.models","r2x_plexos.models"]' |
      r2x run r2x-plexos.plexos-exporter \
        --output-path artifacts/{scenario}/{solve_year}/plexos \
        --model-name Geothermal_{scenario} \
        --horizon-year {solve_year} \
        --weather-year {weather_year} \
        --template PLEXOS10.0
    input_files:
      - parsed_system_{scenario}_{solve_year}_{weather_year}
```

`parsed_system_*` tracks the JSON entrypoint. Its adjacent `infrasys_time_series/` directory is part of the same r2x System bundle and must remain co-located on the shared filesystem.

## Fan in parameterized outputs

A parameterized consumer creates one consumer per parameter combination. For one job that consumes all matching producer files, use `input_file_regexes` instead:

```yaml
- name: aggregate_results
  command: python aggregate.py --input-dir=/results --output=/results/summary.csv
  input_file_regexes:
    - '^metrics_lr.*$'
```
