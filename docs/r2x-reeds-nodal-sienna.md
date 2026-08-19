# ReEDS Nodal Sienna Pipeline (Zonal to Nodal)

This pipeline takes a zonal ReEDS result and a nodal Sienna system and produces a nodal Sienna system with ReEDS capacity expansion results mapped to individual buses.

## Required Plugins

```bash
r2x install r2x-reeds
r2x install r2x-sienna
r2x install r2x-nodal
```

## Pipeline YAML

```yaml
variables:
  sienna_system: <path_to_nodal_system>.json
  reeds_run: <reeds_run_path>
  output_dir: <output_path>
  solve_year: 2023
  weather_year: 2012

pipelines:
  r2s:
    - r2x-sienna.sienna-parser
    - r2x-reeds.reeds-parser
    - r2x-reeds.break-gens
    - r2x-reeds.add-pcm-defaults
    - r2x-nodal.zonal-to-nodal
    - r2x-sienna.sienna-exporter

config:
  r2x-sienna.sienna-parser:
    json_path: ${sienna_system}

  r2x-reeds.reeds-parser:
    weather_year: ${weather_year}
    solve_year: ${solve_year}
    path: ${reeds_run}

  r2x-reeds.break-gens:
    drop_capacity_threshold: 5
    reference_units: <path_to_pcm_defaults.json>
    break_category: technology
    skip_categories:
      - nuclear

  r2x-reeds.add-pcm-defaults:
    pcm_defaults_fpath: <path_to_pcm_defaults.json>
    pcm_defaults_override: true

  r2x-nodal.zonal-to-nodal:
    name: r2x_example
    output_folder: ${output_dir}
    load_year: ${solve_year}
    overwrite: true
    nodal_system: ${sienna_system}
    cem_system_path: ${reeds_run}
    enable_revx: false
    revx_config:
      revx_techs:
        - upv
        - wind-ons
        - wind-ofs
      capacity_threshold: 100
      voltage_threshold: 100
    transmission:
      add_transmission: false
      disable_capacity_limits: true
      universal_line_addition_limit: 4
      rating_selection: sil_MW  # "MVA" or "sil_MW" (surge impedance loading)
      distance_min: 2.0
      voltage_classes: [345, 500, 765]
      centrality_weight: 0.5  # balance centrality vs. distance CAPEX
      centrality_cutoff_num: 3
    transmission_visualization:
      plot_interconnect_list:
        - "texas"
    build_capacity_limit: 2000
    geometry_fname: maps.gpkg
    geometry_fpath: ${reeds_run}/inputs_case/maps.gpkg
    gpkg_key: "r"
    unit_build_limit: 50
    excluded_techs: null

  r2x-sienna.sienna-exporter:
    output_path: ${output_dir}/<output_filename>.json

output_folder: ${output_dir}
```

## Running

```bash
r2x run zonal-to-nodal-pipeline.yaml r2s
```

## Entry Details

### `r2x-sienna.sienna-parser`

- `json_path`: required path to the nodal Sienna system JSON file.

### `r2x-reeds.reeds-parser`

- `path`: ReEDS run folder used by the parser data store.
- `solve_year`: required; supports `int` or `list[int]`.
- `weather_year`: required; supports `int` or `list[int]`.

### `r2x-reeds.break-gens`

- `drop_capacity_threshold`: capacity (MW) below which generators are dropped.
- `reference_units`: optional path to a PCM defaults JSON to inform unit splitting.
- `break_category`: grouping key for splitting (e.g. `technology`).
- `skip_categories`: list of technology categories to skip during breaking.

### `r2x-reeds.add-pcm-defaults`

- `pcm_defaults_fpath`: path to a PCM defaults JSON file.
- `pcm_defaults_override`: when `true`, overrides existing properties.

### `r2x-nodal.zonal-to-nodal`

- `name`: label for the output run artifact.
- `output_folder`: output directory.
- `load_year`: year used to select load profiles.
- `overwrite`: when `true`, overwrites any existing output.
- `nodal_system`: path to the nodal Sienna JSON used as the network backbone.
- `cem_system_path`: path to the ReEDS run folder for capacity expansion data.
- `enable_revx`: when `true`, uses reVX for renewable siting.
- `revx_config`: sub-block for reVX parameters (techs, capacity/voltage thresholds).
- `transmission.add_transmission`: add new transmission lines to the nodal network.
- `transmission.disable_capacity_limits`: ignore transmission capacity limits.
- `transmission.rating_selection`: rating basis — `"MVA"` or `"sil_MW"`.
- `transmission.voltage_classes`: list of kV classes to include for new lines.
- `transmission.centrality_weight`: weight between centrality and distance CAPEX (0–1).
- `transmission.distance_min`: minimum line distance (km) to consider.
- `build_capacity_limit`: maximum capacity (MW) that can be built per bus.
- `unit_build_limit`: maximum number of discrete units per bus.
- `geometry_fname`: GeoPackage filename for bus geometry.
- `geometry_fpath`: full path to the GeoPackage file.
- `gpkg_key`: key column in the GeoPackage.
- `excluded_techs`: list of technology categories to exclude from nodal mapping.

### `r2x-sienna.sienna-exporter`

- `output_path`: required; output Sienna JSON file path.
- `system_base_power`: optional base MVA; default `100.0`.
- `skip_validation`: optional; default `false`.

## Validation Commands

```bash
# list available pipelines
r2x run zonal-to-nodal-pipeline.yaml --list

# print resolved config
r2x run zonal-to-nodal-pipeline.yaml --print r2s

# preview without executing
r2x run zonal-to-nodal-pipeline.yaml r2s --dry-run
```
