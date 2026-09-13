<p align="center">
  <img src="https://raw.githubusercontent.com/anibalanto/accreta/main/images/bilinker.png" alt="Bilinker" width="200"/>
</p>

Bilinker mantiene referencias bidireccionales y verificadas entre fragmentos de texto en distintas capas de un proyecto: la spec, las decisiones, el código. La referencia apunta a un nodo del AST vía tree-sitter y no a un número de línea, así que sobrevive reformateos y movimientos; y cada fragmento lleva el hash de lo que alguien aprobó, así que cuando cambia se nota y alguien tiene que volver a aprobarlo.

La especificación vive acá, en [`docs/specs/`](docs/specs/): un archivo por concepto en [`concepts/`](docs/specs/concepts/), y los comandos en [`commands.md`](docs/specs/commands.md). Las decisiones abiertas están en [`docs/decisions/`](docs/decisions/); los ADR numerados de [`docs/adr/`](docs/adr/) son historia.

## Instalación

```bash
cargo install --path crates/bilinker-cli
```

Después, en cada clon donde se vaya a usar:

```bash
bilinker init
```

## Lo que no es

Opera sólo con git y tree-sitter. No consulta language servers ni indexers por su cuenta: el grafo del proyecto es de lattice, el alcance de un cambio es de impact, y el vecindario de una firma se lo pide a `lspd` por un puerto que no nombra a nadie.

## Crates

| | |
|---|---|
| `bilink-format` | los tipos y su serialización: el formato, y su versión es la del formato |
| `bilink-format-v1` | el lector congelado del formato 1, para migrar |
| `bilink-migrate` | las migraciones entre formatos |
| `accreta-migrate` | el mecanismo general: ledger por repo, migraciones por capa |
| `bilinker` | la librería: tree-sitter, git, estados, la ref |
| `bilinker-cli` | el binario `bilinker` |
| `bilinker-lsp` | el language server: `hover` y `codeLens` sobre los bilinks del archivo abierto |
