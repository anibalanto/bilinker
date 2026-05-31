<p align="center">
  <img src="https://raw.githubusercontent.com/anibalanto/accreta/main/images/bilinker.png" alt="Bilinker" width="200"/>
</p>

Bilinker crea referencias bidireccionales y persistentes entre fragmentos de texto en distintas capas de un proyecto. Los links sobreviven reformateos, renombres y movimientos de bloques porque se anclan en la estructura AST, no en números de línea.

## El problema

Las referencias por número de línea se rompen en cualquier inserción o reformateo. En proyectos con múltiples capas (spec → ADR → código), el drift entre lo que dice la spec y lo que hace el código se acumula silenciosamente. Bilinker lo detecta y lo hace explícito.

## Instalación

```bash
cargo install --path crates/bilinker-cli
```

Requiere que la dependencia `stratum` esté disponible (ver `Cargo.toml`).

## Conceptos

### Bilink

Un bilink conecta dos fragmentos. Vive en `.bilink/<uuid>.bilink`:

```
link.0: commands/tree.md
link.1: >impl

hash.0: 282bccc8b075...
commit.0: 4d02a5ba951a
hash.1: a6706965fd47...
commit.1: 8c650cb74d1b
state.0: OK
state.1: OK
resolved_at: 2026-05-30T18:00:00Z
```

El mismo UUID aparece en todas las capas que participan del link. El nombre del archivo es el identificador.

### Tipos de endpoint

| Tipo | Ejemplo | Descripción |
|------|---------|-------------|
| **Estructural** | `src/Persona.java :: (class_declaration ...)` | Fragmento de archivo identificado por query AST |
| **Layer** | `>impl` o `../..` | Nodo adyacente en otra capa (path Stratum) |
| **Archivo completo** | `commands/tree.md` | El archivo entero como fragmento |
| **Task** | `task 3a` | Ítem del worklist |
| **Bilink** | `.bilink/<uuid>.bilink` | Otro bilink (para links de impacto) |

### Cadena entre capas

Un bilink conecta dos capas a través de un UUID compartido. Cada nodo es un archivo `.bilink` con el mismo UUID:

```
spec/voting.yaml  ←→  tip(spec)  ←→  tip(impl)  ←→  Persona.java
```

Cuando cambia `Persona.java`, `bilinker check` lo detecta → `ALTERED`. Al aceptar el cambio, el nodo adyacente detecta `CHAIN_DIRTY` en el próximo check. La propagación es explícita, hop a hop.

### Estados

**Endpoint estructural:**

| Estado | Significado |
|--------|-------------|
| `PENDING` | No aceptado aún |
| `OK` | Hash actual == hash aceptado |
| `ALTERED` | Contenido cambió, requiere aceptación |
| `DISPLACED` | Hash encontrado en posición diferente del mismo nodo AST |
| `MOVED` | Archivo renombrado, hash encontrado en nueva ruta |
| `UNANCHORED` | La query AST ya no matchea ningún nodo |
| `DELETED` | El archivo fue eliminado |

**Endpoint layer:**

| Estado | Significado |
|--------|-------------|
| `PENDING` | La capa existe pero no fue aceptada |
| `TODO` | La capa apuntada aún no existe |
| `OK` | Hash del vecino estructural == hash almacenado |
| `CHAIN_DIRTY` | El vecino estructural fue re-aceptado con contenido nuevo |
| `BROKEN` | La capa desapareció |

## Comandos

### `bilinker chain new` — crear un bilink

```bash
bilinker chain new \
  --tip "commands/tree.md" \
  --tip ">impl/crates/estrato-cli/src/main.rs:29:1"
```

Genera un UUID, crea el archivo `.bilink` en ambos lados. Los endpoints sin `:LINE:COL` capturan el archivo completo.

```bash
# Con nodo intermedio
bilinker chain new \
  --tip "spec/voting.yaml:42:1" \
  --mid ">tech-decisions" \
  --tip ">impl/src/Voting.java:10:1"
```

### `bilinker capture` — generar query AST desde una selección

```bash
bilinker capture src/Persona.java 11:5 13:10
```

Usa tree-sitter para encontrar el nodo AST más estable que contiene la selección. Imprime la query lista para usar en un `link.N`.

### `bilinker check` — verificar consistencia

```bash
bilinker check .
```

Compara el contenido actual de cada fragmento contra el hash almacenado. Actualiza `state.N` y `resolved_at`.

```
all clean (6 bilink(s))
```

```
38962614  (OK, ALTERED)
7f3d8e9a  (CHAIN_DIRTY, OK)
```

Exit 0 si todo está en `OK`/`MOVED`/`DISPLACED`. Exit 1 si hay estados que requieren atención.

### `bilinker accept` — establecer el estado actual como aceptado

```bash
# Aceptar un endpoint específico
bilinker accept 38962614-a28d-4e13-9e0f-4055e05b7e57.0

# Aceptar ambos endpoints del bilink
bilinker accept 38962614-a28d-4e13-9e0f-4055e05b7e57

# Aceptar todos los PENDING en el directorio actual
bilinker accept .
```

Establece `hash.N` al SHA-256 del contenido actual y `commit.N` al HEAD del repo.

### `bilinker get` — navegar y leer fragmentos

```bash
# Ver el contenido del fragmento de un endpoint
bilinker get "38962614-a28d-4e13-9e0f-4055e05b7e57.0"
# → commands/tree.md  lines 1–51
# → # Especificación: `stratum tree` ...

# Ver todos los bilinks que referencian un archivo
bilinker get "commands/tree.md"
# → 38962614.0  >impl  bytes 0–1553

# Ver bilinks en una posición concreta
bilinker get "src/Persona.java:42:5"
```

### `bilinker status` — resumen del estado actual

```bash
bilinker status
```

```
commands/
  add.md      125971da  (OK, OK)
  estrato.md  8e2e749a  (OK, OK)
  tree.md     38962614  (OK, OK)

concepts/
  layer-model.md      6d8b8502  (OK, OK)
  paths.md            9cfe0db7  (OK, OK)
  sublayer-config.md  35cbef68  (OK, OK)
```

### `bilinker apply` — aplicar auto-fixes

Los estados `MOVED`, `DISPLACED`, `REANCHORED` y `EXPANDED` tienen fixes determinísticos:

```bash
bilinker apply           # interactivo
bilinker apply -y        # sin confirmación
bilinker apply --dry-run # solo mostrar
```

### `bilinker watch` — monitorear cambios en tiempo real

```bash
bilinker watch
# ALTERED  src/Persona.java  chain 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a
```

### `bilinker index` — índice de lookups O(1)

```bash
bilinker index build           # construir índice en capa actual
bilinker index build --recursive  # construir en todas las capas
bilinker index status          # verificar si está actualizado
```

## Flujo de trabajo típico

### 1. Crear un bilink spec ↔ impl

```bash
# Desde la spec layer
cd mi-proyecto/

bilinker chain new \
  --tip "specs/voting.yaml" \
  --tip ">impl/src/Voting.java:10:1"

# Poblar la cache
bilinker check .

# Aceptar ambos extremos
bilinker accept .
cd .stratum/impl/
bilinker accept .
```

### 2. Detectar y resolver drift

```bash
# Alguien modifica impl/src/Voting.java

bilinker check .
# → 7f3d8e9a  (OK, ALTERED)

# Revisar qué cambió, luego aceptar
bilinker accept 7f3d8e9a.1

# En la spec layer, el check detectará CHAIN_DIRTY
cd ../..
bilinker check .
# → 7f3d8e9a  (CHAIN_DIRTY, OK)

bilinker accept 7f3d8e9a.0
```

### 3. Navegar bidireccionalmente

```bash
# ¿Qué spec está vinculada a esta línea de código?
bilinker get "src/Voting.java:42:5"
# → 7f3d8e9a.1  specs :: voting.yaml  bytes 312~358

# Ver el fragmento de spec correspondiente
bilinker get "7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.0"
# → specs/voting.yaml  lines 18–24
# → impl: ...
```

## Integración con el ecosistema

```
bilinker     ← detecta drift entre capas linkedeadas
impact       ← analiza el alcance, abre hilos de discusión
worklist     ← registra el trabajo concreto pendiente
stratum      ← provee el lenguaje de paths para endpoints layer
```

Los endpoints layer usan paths Stratum (`>impl`, `<`, `<*`, etc.) para referenciar capas adyacentes sin hardcodear rutas absolutas.
