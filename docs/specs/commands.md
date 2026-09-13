# Comandos

Una fila por comando. El detalle de cada uno está en el concepto al que remite: la fila es el ancla, el concepto dice la regla.

| Comando | Qué hace | Concepto |
|---|---|---|
| `bilinker capture <file> [<l>:<c> <l>:<c>]` | Crea un capture a partir de una selección: sube en el AST hasta el primer ancestro estable y escribe la ubicación. Sin selección, el archivo entero. | [capture](concepts/capture.md) |
| `bilinker capture prune` | Borra los captures que ningún bilink referencia, ni como `link` ni como vecino. | [capture](concepts/capture.md) |
| `bilinker capture remove <id>` | Borra un capture sin referentes; se niega si alguno lo referencia. | [capture](concepts/capture.md) |
| `bilinker recapture <uuid>.<N> <file> [<l>:<c> <l>:<c>]` | Repunta un endpoint estructural a otro fragmento, a mano. No acepta. | [capture](concepts/capture.md) |
| `bilinker get <file>[:<l>:<c>]` · `get <uuid>.<N> [--diff] [--raw]` | Navega: qué endpoints cubren una posición o un archivo, y qué texto referencia un endpoint, con su vecindario. | [get](concepts/get.md) |
| `bilinker check [<path>] [--against <ref>]` | Verifica la capa: resuelve captures, compara contra `accepted` y escribe la cache. No escribe nada versionado. | [check](concepts/check.md) |
| `bilinker status` | Muestra la cache agrupada por archivo, sin re-verificar. | [check](concepts/check.md) |
| `bilinker watch` | Reporta en tiempo real los archivos vinculados que se modifican. | [check](concepts/check.md) |
| `bilinker accept <uuid>[.<N>] \| . [--place\|--content] [--no-n1] [--force]` | Escribe `accepted`: la única decisión del formato. Absorbe la rama y commitea en la ref, un commit por endpoint. | [accept](concepts/accept.md) |
| `bilinker apply [--dry-run] [-y]` | Repunta los `link` de los captures `MOVED` y `REANCHORED` acuñando el capture nuevo. Propone; nunca escribe `accepted`. | [apply](concepts/apply.md) |
| `bilinker chain new --tip <REF> --tip <REF> [--as <modo>]` | Crea una cadena: un UUID y un bilink en cada capa que los tips atraviesan. | [chain](concepts/chain.md) |
| `bilinker chain status <uuid>` | Recorre todos los nodos de una cadena con su estado. | [chain](concepts/chain.md) |
| `bilinker chain list [--kind] [--link] [--as]` | Lista las cadenas a partir del directorio actual, con filtros que se combinan con Y. | [chain](concepts/chain.md) |
| `bilinker remove <uuid>` | Elimina el bilink de la capa actual. Los vecinos detectan `BROKEN` en el próximo `check`. | [chain](concepts/chain.md) |
| `bilinker graph <selector> [--format tree\|flat\|json] [--depth <n>] [--recursive]` | Recorre el grafo de bilinks cruzando capas. `json` es el contrato de proveedor hacia lattice. | [graph](concepts/graph.md) |
| `bilinker index [--recursive]` · `index status` | Construye el índice derivado de la capa, o dice si está al día. | [index](concepts/index.md) |
| `bilinker init [--dry-run]` | Pone a punto el clon: exclusión, refspec y `.bilink/` materializado. Lo primero en un clon nuevo. | [ref](concepts/ref.md) |
| `bilinker sync [--dry-run]` | Absorbe el tip de la rama del proyecto en la ref, sin decidir nada. | [ref](concepts/ref.md) |
| `bilinker track <branch> [--from <rama>]` | Crea `refs/bilink/<branch>` para una rama que no la tiene, heredando de la ref de la que sale; sin candidato, es el corte. | [ref](concepts/ref.md) |
| `bilinker adopt <rama> [--dry-run]` | Trae a la ref de esta rama las decisiones que otra rama aceptó. | [ref](concepts/ref.md) |
| `bilinker pull [<remote>] [--dry-run]` | Trae lo que otro aceptó en la misma rama y lo une con lo propio. | [ref](concepts/ref.md) |
| `bilinker push [<branch>] [<remote>]` | Publica `refs/bilink/<branch>` en el remoto. `git push` no la empuja. | [ref](concepts/ref.md) |
| `bilinker verify-ref [<ref>]` | Verifica que una `refs/bilink/*` tenga la forma que la ref promete: fidelidad, disyunción, un acto por commit, el mensaje y las firmas. Corre en un repo desnudo. | [ref](concepts/ref.md) |
| `bilinker history <uuid>[.<N>]` | Qué le pasó a un bilink: quién aceptó qué, cuándo y contra qué código. | [ref](concepts/ref.md) |
| `bilinker log [--first-parent <rama> ^<rama>]` | El registro de decisiones: los commits propios de la ref. | [ref](concepts/ref.md) |
| `bilinker diff [--against <ref>]` | `.bilink/` contra el commit de la ref del que salió. Vacío quiere decir que todo lo aceptado está en la ref. | [ref](concepts/ref.md) |
| `bilinker relayer <capa>` | Mueve los bilinks de una capa a la de arriba, reacuñando sus captures. | [ref](concepts/ref.md) |
| `bilinker fetch [<alias>]` | Trae el repo de un proveedor declarado en `.bilink/.<alias>.toml`: su ref de bilinks, superficial y con el sparse calculado. | [frontier](concepts/frontier.md) |
| `bilinker abstracts [<alias>]` | Qué abstracciones publica un proveedor, con su código. Sin alias, las de esta capa. | [frontier](concepts/frontier.md) |
| `bilinker migrate [<path>] [--recursive] [--dry-run]` | Migra los metadatos de una capa al formato vigente, en un path transitorio hasta el corte. | [migration](concepts/migration.md) |
| `bilinker restore-n1 [<path>] [--recursive] [--dry-run] [--from <dir>]` | Devuelve el vecindario que la migración `003` descartó, leyéndolo del backup del corte. No es una migración. | [migration](concepts/migration.md) |
| `bilinker-lsp` | El language server: `hover` y `codeLens` sobre los bilinks del archivo abierto, por stdio. | [lsp](concepts/lsp.md) |
