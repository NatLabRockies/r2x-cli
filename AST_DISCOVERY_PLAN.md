# AST-Based Plugin Discovery Implementation Plan

## Overview
Replace Python interpreter-based plugin discovery with Rust-based AST parsing using `ast-grep` to achieve 227x speedup while maintaining 100% output compatibility.

## Architecture

### Phase 1: AST Extraction (Current)
Use ast-grep to extract plugin definitions from Python source:

```
plugins.py
  ├── register_plugin() function
  │   ├── Imports (for module path resolution)
  │   └── Package() constructor call
  │       └── plugins=[...] array
  │           ├── ParserPlugin(name=..., obj=..., ...)
  │           ├── UpgraderPlugin(...)
  │           └── BasePlugin(...)
```

### Phase 2: Import Resolution
Build a symbol table mapping identifiers to full module paths:
- `ReEDSParser` → `r2x_reeds.parser:ReEDSParser`
- `IOType.STDOUT` → handle enum access
- `UpgraderPlugin.steps` → handle attribute access (defer to class inspection)

### Phase 3: JSON Serialization
Transform extracted AST into Pydantic JSON matching `model_dump_json()`:
- Map plugin types to discriminator values
- Serialize `Importable` fields as `module:name` strings
- Handle nested structures (CallableMetadata, ConfigMetadata, etc.)
- Convert Python enums to string values

### Phase 4: Validation & Integration
- Compare ast-generated JSON with Python output
- Add experimental feature flag (`--experimental-ast-discovery`)
- Fallback to Python on extraction failure
- Full integration when validated

## AST-Grep Rules

### Rule 1: Extract Package Constructor
```yaml
id: package_constructor
language: python
rule:
  pattern: |
    Package(
      name=$NAME,
      plugins=$PLUGINS
    )
```

### Rule 2: Extract Plugin Constructors
```yaml
id: plugin_constructors
language: python
rule:
  pattern: |
    $PLUGIN_TYPE(
      $$$
    )
```

### Rule 3: Extract Imports
```yaml
id: function_imports
language: python
rule:
  pattern: |
    def register_plugin():
      $$$
      from $MODULE import $NAMES
```

## Implementation Phases

### Phase 1A: Core AST Module (✅ ast_discovery.rs)
- Entry point: `AstDiscovery::extract_from_entry_point(package_path, entry_module)`
- Uses ast-grep to find and parse plugins.py
- Returns structured plugin data before JSON serialization

### Phase 1B: Import Resolution (IN PROGRESS)
- Parse `from X import Y` statements
- Build symbol→(module, name) map
- Handle attribute access (IOType.STDOUT → enum value)

### Phase 2: JSON Builder
- Convert extracted data to Pydantic-compatible JSON
- Match CallableMetadata, ConfigMetadata structures
- Handle union types (ParserPlugin | UpgraderPlugin | BasePlugin)

### Phase 3: Integration
- Add to discovery.rs as alternative path with `--experimental-ast-discovery`
- Implement fallback to Python on AST parse failure
- Validation tests comparing outputs

## Edge Cases & Handling

1. **Forward References**: Users specify explicitly in doc (skip those)
2. **Enum Access**: `IOType.STDOUT` → extract enum value
3. **Attribute Access**: `UpgraderPlugin.steps` → need class inspection
4. **Dynamic Values**: Fail gracefully and fallback to Python
5. **Complex Nested Structures**: Parse carefully or fallback

## Testing Strategy

1. **Unit Tests**: Test each extraction rule with known inputs
2. **Integration Tests**: Compare ast output vs Python for reeds, plexos, etc.
3. **Feature Flag**: `--experimental-ast-discovery` for user opt-in
4. **Validation**: Always compare both and warn if mismatches

## Success Criteria

- ✅ Extract all plugin types (ParserPlugin, BasePlugin, UpgraderPlugin)
- ✅ Resolve imports correctly
- ✅ Generate JSON matching Python's `model_dump_json()` exactly
- ✅ 227x speedup in plugin discovery
- ✅ No Python interpreter startup
- ✅ Works with editable installs via UV

## Files to Create/Modify

### New Files
- `src/plugins/ast_discovery.rs` - Main AST extraction module
- `src/plugins/ast_discovery/import_resolver.rs` - Import resolution
- `src/plugins/ast_discovery/json_builder.rs` - JSON serialization
- `tests/ast_discovery_tests.rs` - Integration tests

### Modified Files
- `src/plugins/discovery.rs` - Add AST path with feature flag
- `src/plugins/mod.rs` - Export ast_discovery module
- `Cargo.toml` - Add ast-grep dependency (if needed) or use external process
