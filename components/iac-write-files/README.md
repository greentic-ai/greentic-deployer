# Greentic IaC Write-Files Component

This component implements the `greentic:host/iac-write-files@1.0.0` world and exposes a single operation, `write-files`, that writes a list of textual files inside a host-mounted `/out` directory.

## WIT contract

- **World**: `greentic:host/iac-write-files@1.0.0`
- **FileSpec**: `{ path: string, content: string, overwrite: bool }`
- **WriteError**: `{ code: u32, message: string, path: option<string> }`
- **write-files**: `func(files: list<file-spec>) -> result<list<string>, write-error>`

Rules:

1. All `path` values must be relative paths under `/out`. Absolute paths or any `..` traversal are rejected.
2. Parent directories are created as needed.
3. If `overwrite=false` and the file already exists, the call errors.
4. The function returns the list of written relative paths.

## Mounting `/out`

The host (runner, deployer, or pack generator) decides where `/out` points to on the filesystem. The daemon scripts in this repo default to mounting `/out` to `${repo_root}/dist/iac/<provider>/`, but the path is configurable via the host’s mounting logic. The component never hardcodes the mount target.

## Examples

```rust
let specs = vec![
    FileSpec {
        path: "README.md".into(),
        content: "placeholder".into(),
        overwrite: false,
    },
    FileSpec {
        path: "iac.placeholder".into(),
        content: "provider: local".into(),
        overwrite: true,
    },
];
write_files(&specs, Path::new("/out")).expect("files written");
```
