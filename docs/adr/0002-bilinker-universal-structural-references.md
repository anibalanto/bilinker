# ADR-0002: bilinker — Referencias estructurales universales y bidireccionales

**Estado:** Propuesto **Fecha:** 2026-05-20 **Actualizado:** 2026-05-24

---

## Contexto

Stratum (ex Genia) define capas de conocimiento que van desde documentación hasta implementación. Actualmente el campo `impl: ref:` usa referencias por número de línea (`"file::start~end"`), que son frágiles ante cualquier refactor.

Se necesita un sistema de referencias que:

- Sea estable ante reformateos, cambios de indentación y movimientos de código.
- Funcione para cualquier lenguaje estructurado: código (Java, Rust, Python, TypeScript,
  Kotlin...), documentos (Markdown, YAML, TOML, JSON, SQL, GraphQL) y cualquier lenguaje con gramática tree-sitter.
- Permita referenciar fragmentos con granularidad arbitraria: una función completa,
  una lambda dentro de un método, un párrafo bajo un heading, un campo YAML.
- Detecte consistencia: si el fragmento referenciado cambia, el link lo sabe.
- Sea bidireccional: dado un nodo, se puede saber quién lo referencia.

---

## Decisión

### 1. bilinker como herramienta independiente

bilinker es una librería y CLI independiente de Accreta, Stratum y expancode. Es infraestructura — sus consumidores son:

- **Stratum** — reemplaza `impl: ref: "file::start~end"` por referencias bilinker
- **expancode** — usa bilinker para expandir inline el fragmento referenciado
- **Accreta** — usa bilinker para detectar drift entre capas (spec ↔ código ↔ tests)

### 2. Anatomía de una referencia bilinker

Una referencia bilinker tiene hasta cuatro componentes; `query` y `start~end` son opcionales:

```
workspace :: file [:: query [:: start~end]]
```

```
[workspace]          →  dónde buscar y con qué gramática tree-sitter
  [file]             →  qué archivo dentro del workspace
    [query]          →  (opcional) nodo AST dentro del archivo (âncora estructural)
      [start~end]    →  (opcional) sub-fragmento en bytes relativo al nodo
```

Cuando `query` se omite, el endpoint apunta al **archivo completo**: `hash.N` es el SHA-256 del archivo íntegro y no se genera `range.N`. Útil para referenciar un documento o módulo entero sin anclar a un nodo específico.

**`workspace`** — nombre definido en `.bilinker.toml` que provee el directorio raíz y el lenguaje (gramática tree-sitter) para parsear los archivos.

```toml
# .bilinker.toml
[workspaces.java-demo]
path     = "java-app"
language = "java"

[workspaces.specs]
path     = "specs"
language = "yaml"

[workspaces.docs]
path     = "docs"
language = "markdown"

[workspaces.kotlin-demo]
path     = "kotlin-app"
language = "kotlin"
```

Cuando bilinker se usa dentro de un proyecto expancode, puede leer los workspaces de `.expancode.toml` directamente; solo necesita `language` como campo adicional.

**`file`** — ruta relativa a la raíz del workspace. Es parte de la identidad: distingue clases homónimas en distintos paquetes y aplica igual a código y documentos.

**`query`** — S-expression tree-sitter que identifica el nodo contenedor. Se construye subiendo en el AST desde la selección hasta el primer ancestro con nombre estable (función, clase, heading, campo). Ejemplos:

```scheme
; Método en Java  (cada captura tiene nombre único @n0, @n1, …)
(class_declaration
  name: (identifier) @n0 (#eq? @n0 "Persona")
  body: (class_body
    (method_declaration
      name: (identifier) @n1 (#eq? @n1 "vote")) @target))

; Párrafo bajo un heading en Markdown
(section
  (atx_heading
    (inline) @n0 (#eq? @n0 "Decisión de arquitectura"))
  (paragraph) @target)

; Campo en YAML
(block_mapping_pair
  key: (flow_node) @n0 (#eq? @n0 "impl")
  value: (_) @target)
```

Los nombres de captura (`@n0`, `@n1`, …) son únicos por query para evitar el error "Impossible pattern" de tree-sitter con capturas repetidas.

**`start~end`** — (opcional) offsets en bytes relativos al inicio del nodo matcheado por la query. Permite referenciar un sub-fragmento dentro del nodo: una oración dentro de un párrafo, una expresión dentro de un método.

Si se omite, la referencia apunta al nodo AST completo.

Si se incluye, es un hint auto-corregible: cuando el hash del fragmento se encuentra en un offset diferente al guardado (texto insertado antes de la selección dentro del mismo nodo), bilinker actualiza `start~end` en el `.bilink` silenciosamente.

**`hash`** (en cache) — SHA-256 del texto exacto del fragmento seleccionado. Es la fuente de verdad: si el hash no matchea en ninguna posición dentro del nodo, el link está genuinamente roto.

### 3. Formato del archivo `.bilink` y modelo de cadenas

Un bilink conecta exactamente dos extremos (`link.0` y `link.1`). Los bilinks se encadenan entre layers para conectar fragmentos estructurales a través de las capas del proyecto. El nombre del archivo es un **UUID v4** que identifica la cadena.

#### Dos tipos de endpoint

| Tipo | Forma | Distinguido por |
|---|---|---|
| **Estructural** | `workspace :: file :: query [:: start~end]` | contiene `::` |
| **Layer** | `<ruta-relativa-a-layer>` | sin `::` |

Un endpoint layer resuelve al archivo `.bilink/<uuid>.bilink` de esa layer. La carpeta `.bilink/` nunca aparece en el valor de `link.N` — es implícita:

```
link.N: <layer-path>  →  ../<layer-path>/.bilink/<uuid>.bilink
```

#### Topología de cadena

Una **cadena** es una secuencia lineal de bilinks con el mismo UUID:

- **tip**: un endpoint estructural + un endpoint layer (extremos de la cadena).
- **mid**: ambos endpoints son layer (nodos intermedios).

```
[fragmento] ←→ tip ←→ mid* ←→ tip ←→ [fragmento]
```

#### Formato del archivo

```
link.0: <referencia-estructural-o-layer>
link.1: <referencia-estructural-o-layer>

# --- cache generada por bilinker, no editar a mano ---
hash.0: <sha256-hex-64-chars>
range.0: <start~end-bytes-absolutos>
hash.1: <sha256-hex-64-chars>
range.1: <start~end-bytes-absolutos>
state.0: <estado>
state.1: <estado>
resolved_at: <iso8601-utc>
```

No existe campo `id` — el UUID del nombre de archivo es el identificador. `range.N` solo aparece para endpoints estructurales.

**`hash.N`:**
- Endpoint estructural: SHA-256 del texto exacto del fragmento.
- Endpoint layer: SHA-256 del archivo `.bilink` completo de esa layer.

**`range.N`:** byte range absoluto del fragmento en su archivo fuente. Calculado y actualizado por `check`. Permite que `bilinker get <file>:<line>:<col>` localice qué endpoints cubren una posición sin necesidad de re-ejecutar las queries tree-sitter.

**`state.N`:** estado de consistencia del endpoint, persistido en el archivo. Al cambiar `state.N`, el archivo cambia y su hash cambia — esto propaga el cambio al nodo adyacente de la cadena en el próximo `check` (propagación reactiva).

#### Ejemplo: cadena de 3 nodos (spec → tech-decisions → impl)

```
# .bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (tip — spec layer)
link.0: specs :: voting.yaml :: (block_mapping_pair
  key: (flow_node) @n0 (#eq? @n0 "impl")
  value: (_) @target)
link.1: .stratum/tech-decisions

# --- cache generada por bilinker, no editar a mano ---
hash.0: a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2
range.0: 312~358
hash.1: e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6
state.0: OK
state.1: OK
resolved_at: 2026-05-24T10:00:00Z
```

```
# .stratum/tech-decisions/.bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (mid)
link.0: ../..
link.1: ../impl

# --- cache generada por bilinker, no editar a mano ---
hash.0: c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0
hash.1: a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4
state.0: OK
state.1: OK
resolved_at: 2026-05-24T10:00:00Z
```

```
# .stratum/impl/.bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (tip — impl layer)
link.0: ../tech-decisions
link.1: java-demo :: src/main/java/ar/example/demo/persona/Persona.java :: (class_declaration
  name: (identifier) @n0 (#eq? @n0 "Persona")
  body: (class_body
    (method_declaration
      name: (identifier) @n1 (#eq? @n1 "vote")) @target))

# --- cache generada por bilinker, no editar a mano ---
hash.0: e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8
hash.1: 479922a1ee55cc7f9f4f323bb002018e1b4e1cda65e069e0f6f4645926ce25ee
range.1: 245~389
state.0: OK
state.1: OK
resolved_at: 2026-05-24T10:00:00Z
```

### 4. Semántica de consistencia — diez estados

`check` analiza cada extremo independientemente y devuelve una **tupla de estados** `(state_link0, state_link1)`. Ejemplo: `(ALTERED, OK)` o `(OK, CHAIN_DIRTY)`.

Git es dependencia dura de `check` y `apply`. `capture` y `get` funcionan sin git.

#### Estados para endpoints estructurales (9 estados)

| Estado | Condición | Auto-fix |
|---|---|---|
| **OK** | hash matchea en offset guardado | — |
| **MOVED** | archivo cambió de path (git rename `-M`), hash matchea en nuevo path | ✓ actualiza `file` en `link.N` |
| **DISPLACED** | query matchea, hash matchea en offset diferente dentro del nodo | ✓ actualiza `start~end` |
| **REANCHORED** | anchor renombrado/movido; se detecta su nueva posición vía git + AST | ✓ actualiza predicados de la query |
| **EXPANDED** | fragmento creció, sin cambio estructural interno (AST interno igual) | ✓ amplía `start~end` |
| **UNANCHORED** | query no matchea y no se detecta nueva posición del anchor | — requiere intervención |
| **ALTERED** | fragmento encontrado, AST interno cambió estructuralmente | — requiere intervención |
| **DELETED** | contenido eliminado de forma determinística (rastreable en git) | — requiere intervención |
| **BROKEN** | ninguna hipótesis aplica; posición indeterminada | — requiere intervención |

#### Estado para endpoints layer (1 estado adicional)

| Estado | Condición | Auto-fix |
|---|---|---|
| **CHAIN_DIRTY** | hash del archivo `.bilink` referenciado no coincide con `hash.N` | — inspeccionar nodo origen |

#### Propagación reactiva via `state.N`

`state.N` se persiste en el archivo `.bilink`. Si el estado cambia, el archivo cambia, su hash cambia, y el nodo adyacente en la cadena detecta CHAIN_DIRTY en el próximo `check`. La cadena es autosuficiente — no requiere índice externo para propagar cambios.

Estados con auto-fix: MOVED, DISPLACED, REANCHORED, EXPANDED. Los fixes se acumulan en `.bilink/.pending/` y se aplican con `bilinker apply` (como commit git con mensaje descriptivo); nunca se aplican silenciosamente.

#### Algoritmo de detección

```
1. ¿El archivo existe en el path conocido?
   NO → git diff -M --name-status → ¿rename detectado con similaridad ≥ 50%?
        SÍ → MOVED  (continuar con nuevo path)
        NO → git log -S "<hash_fragmento>" -- <file> → ¿commit de borrado?
             SÍ → DELETED
             NO → BROKEN

2. Ejecutar la query tree-sitter contra el archivo actual.
   SIN MATCH → buscar el anchor por contenido en el AST actual:
               ¿se encuentra el texto del anchor en otro nodo estable?
               SÍ → REANCHORED  (se registra la nueva query)
               NO → git log -S "<texto_anchor>" -- <file> → ¿commit de borrado?
                    SÍ → DELETED
                    NO → UNANCHORED

3. ¿El hash matchea en el offset guardado?
   SÍ → OK

4. ¿El hash matchea en algún otro offset dentro del nodo?
   SÍ → DISPLACED

5. ¿El texto del fragmento guardado es subcadena del nodo actual?
   SÍ → comparar AST del fragmento guardado vs AST del texto actual en esa posición:
        ASTs estructuralmente iguales  → EXPANDED  (creció sin cambio interno)
        ASTs estructuralmente distintos → ALTERED  (cambio interno)
   NO → git log -S "<hash_fragmento>" -- <file> → ¿commit de borrado?
        SÍ → DELETED
        NO → BROKEN
```

#### Fuente del cambio

bilinker determina el origen del cambio con git para todos los estados no-OK:

| Condición git | Fuente reportada |
|---|---|
| `git diff -- <file>` tiene hunks en el fragmento | `[UNSTAGED]` |
| `git diff --cached -- <file>` tiene hunks en el fragmento | `[STAGED]` |
| `git log --since=<resolved_at> -- <file>` tiene commits | `[commit <hash> "<mensaje>"]` |

`git log -S "<texto_fragmento>" -- <file>` localiza el commit exacto que eliminó o modificó el texto referenciado (usado para DELETED, ALTERED, BROKEN).

#### Intersección hunk / fragmento

```
fragmento: líneas F_start–F_end  (derivadas de start~end en bytes)
hunk:      @@ -H_start,H_count +...

H_start + H_count < F_start  → BEFORE  (causa potencial de DISPLACED)
H_start > F_end               → AFTER   (irrelevante, fragmento no afectado)
se superpone                  → WITHIN  (causa de EXPANDED, ALTERED o REANCHORED)
```

#### Salida de `bilinker check`

`check` muestra la tupla `(state0, state1)` por bilink. Sale con código 0 si todos los estados son `{OK, MOVED, DISPLACED, REANCHORED, EXPANDED}`. Sale con código 1 si algún extremo está en `{UNANCHORED, ALTERED, DELETED, BROKEN, CHAIN_DIRTY}`.

```
$ bilinker check .bilink/

7f3d8e9a  (OK, CHAIN_DIRTY)
  link.1  → .stratum/tech-decisions  archivo cambió
  → inspeccionar: bilinker chain status 7f3d8e9a-...

3a4b5c6d  (DISPLACED, ALTERED)
  link.0  specs::voting.yaml#impl  offset 5~42 → 8~45  [UNSTAGED]
  → fix disponible: bilinker apply
  link.1  java-demo::Persona#vote  AST interno cambió
    - Comparator.comparingInt(String::length)
    + (a, b) -> a.length() - b.length()
    source: commit c7d3e9f "Inline comparator" (2026-05-19)

f1e2d3c4  (EXPANDED, OK)
  link.0  specs::reporter.yaml#generate  fragmento creció — AST sin cambios
    + log.info("called");  [commit a3f2b1c "Add audit log"]
  → fix disponible: bilinker apply
```

### 5. Subcomando `bilinker capture`

Para obtener una referencia desde una selección de texto:

```bash
bilinker capture <workspace> <archivo> <start_line>:<start_col> <end_line>:<end_col>
```

Flujo interno:
1. Determinar el workspace (por nombre o por prefijo del path del archivo).
2. Parsear el archivo con tree-sitter usando la gramática del workspace.
3. Encontrar el nodo más pequeño que contiene la selección.
4. Subir en el AST hasta el primer ancestro con nombre estable.
5. Generar la query como el camino desde ese ancestro hasta el nodo target.
6. Calcular el offset relativo y el hash del fragmento.
7. Imprimir la referencia lista para pegar en un `.bilink`.

```bash
# Selección que coincide exactamente con un nodo AST → sin start~end
$ bilinker capture java-demo src/persona/Persona.java 10:1 18:1
link.N: java-demo :: persona/Persona.java :: (class_declaration
  name: (identifier) @n0 (#eq? @n0 "Persona")
  body: (class_body
    (method_declaration
      name: (identifier) @n1 (#eq? @n1 "vote")) @target))

# Selección parcial dentro de un nodo → con start~end
$ bilinker capture docs architecture.md 34:10 34:52
link.N: docs :: architecture.md :: (section
  (atx_heading (inline) @n0 (#eq? @n0 "Decisión"))
  (paragraph) @target) :: 42~87
```

Cuando la selección cae exactamente en los límites de un nodo AST, `start~end` se omite. Cuando la selección es un sub-fragmento del nodo, `start~end` se incluye automáticamente.

### 6. Subcomando `bilinker check`

Verifica todos los bilinks de un archivo `.bilink` o de un directorio `.bilink/`. Acepta un archivo individual, una carpeta `.bilink/`, o una layer (recursivo):

```bash
bilinker check .bilink/
bilinker check .bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink
```

Ver sección 4 para la semántica completa de estados, algoritmo de detección y formato de salida.

### 6b. Subcomando `bilinker apply`

Aplica los auto-fixes generados por `check` que están en `.bilink/.pending/`. Cada fix se convierte en un commit git con mensaje descriptivo:

```bash
bilinker apply

Pending fixes (2):
  MOVED      7f3d8e9a…  link.1  specs/domain/voting.yaml
  DISPLACED  3a4b5c6d…  link.0  offset 5~42 → 8~45

Apply? [y/N] y

Applied 2 fixes.
Committed: a4b5c6d "bilinker: auto-fix MOVED + DISPLACED (2026-05-24)"
```

Los fixes nunca se aplican automáticamente — siempre requieren `bilinker apply` explícito para que el humano revise antes de confirmar.

### 7. Subcomando `bilinker get`

Navegación bidireccional en tres formas:

**Forma 1: posición → endpoints** — dado un archivo y posición, lista los bilinks cuyo `range.N` la cubre (sin re-ejecutar tree-sitter):

```bash
$ bilinker get src/main/java/ar/example/demo/persona/Persona.java:11:5
7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1   specs :: voting.yaml#impl
```

**Forma 2: endpoint → contenido** — dado un `<UUID>.<N>`, retorna el texto del fragmento en el extremo opuesto:

```bash
$ bilinker get 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1
impl: Persona#vote
description: El método vote registra el voto del ciudadano.
```

**Forma 3: archivo → todos los endpoints** — dado solo un archivo, lista todos los bilinks que lo referencian (por posición o como archivo completo):

```bash
$ bilinker get src/main/java/ar/example/demo/persona/Persona.java
7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1   specs :: voting.yaml#impl          lines 11–18
3a4b5c6d-2e3f-4a5b-9c6d-7e8f9a0b1c2d.1   specs :: persona/voting.yaml#impl  lines 11–18
```

`get` no requiere git ni escribe ningún archivo.

### 8. Carpeta `.bilink/` por layer

Cada layer tiene su propia carpeta `.bilink/` con archivos nombrados por UUID:

```
.bilink/
  7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (tip — spec layer)
.stratum/tech-decisions/
  .bilink/
    7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (mid)
.stratum/impl/
  .bilink/
    7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.bilink   (tip — impl layer)
```

### 9. Lenguajes soportados

Cualquier lenguaje con gramática tree-sitter. Los nodos considerados "âncoras estables" por defecto:

| Tipo de doc | Âncoras estables | Frágil (no recomendado) |
|---|---|---|
| Código | función, método, clase, variable declaration | comentario inline |
| Markdown | heading (h1-h4), code block, list item | párrafo libre |
| YAML / TOML | clave de mapping | valor string libre |
| JSON | clave de objeto | valor primitivo |
| SQL | nombre de tabla, función, vista | expresión inline |

---

## Consecuencias

**Positivas:**
- Estable ante reformateo, cambios de indentación, movimiento de código.
- Universal: mismo mecanismo para código y documentos estructurados.
- El `workspace :: file :: query` en `link.N` es inequívoco: distingue clases del
  mismo nombre en distintos paquetes y aplica igual a código y documentos.
- Formato uniforme para todos los lenguajes — sin casos especiales por lenguaje.
- 4 de 10 estados son auto-reparables (MOVED, DISPLACED, REANCHORED, EXPANDED);
  los 6 restantes requieren intervención humana.
- Los auto-fixes van a staging (`.bilink/.pending/`) — el humano siempre aprueba
  antes de que se apliquen como commit git.
- La tupla de estados `(state0, state1)` por bilink hace explícito qué extremo
  cambió y cuál no, facilitando el análisis de impacto.
- Bidireccional por diseño: `bilinker get <file>:<line>:<col>` retorna todos los endpoints que cubren esa posición.
- Independiente de Accreta/Stratum — se puede usar en cualquier proyecto.

**Negativas:**
- `check` y `apply` requieren git; `capture` y `get` funcionan sin él.
- Requiere que tree-sitter tenga gramática para el lenguaje del archivo referenciado.
- Las âncoras estables dependen del lenguaje — bilinker necesita conocer qué nodos
  son "estables" para cada gramática.
- `range.N` debe mantenerse actualizado — si un `.bilink` nunca pasa por `check`, `get <file>:<line>:<col>` puede no encontrar ese endpoint.

---

## Relación con ADR-0001

ADR-0001 define el campo `link:` en expancode usando LSP para resolver símbolos. bilinker es complementario: donde LSP resuelve símbolos en tiempo real contra un language server, bilinker registra referencias estructurales persistentes con consistencia verificable. A largo plazo, `expancode symbol` podría generar referencias bilinker en lugar de referencias LSP puras.

---

## Alternativas descartadas

- **`file:line` (GeniaSpec actual)** — frágil ante cualquier cambio de línea.
- **Solo hash del archivo completo** — demasiado grueso; cualquier cambio en el
  archivo invalida todas las referencias.
- **Solo query tree-sitter sin hash** — detecta si el nodo existe pero no si su
  contenido cambió.
- **SCIP / LSP únicamente** — resuelven símbolos pero no fragmentos arbitrarios
  (una lambda, un párrafo, una selección parcial dentro de una función).
- **`path.N` en la cache** — redundante si el path ya está en `link.N` como parte
  de la identidad. La ruta absoluta se calcula siempre como `workspace.path + link.N.file`.
- **Mono-links (un solo endpoint)** — rompen la cadena de hashes bidireccional. Un bilink
  que referencia a otro bilink mediante `link.N: path/to/other.bilink` permite trackear el hash del archivo referenciado, pero un mono-link no tiene `link.N` para que el nodo adyacente cierre el ciclo reactivo. Descartado en favor del modelo de cadenas con UUID.
- **Índice SQLite centralizado** — a la escala de bilinker (decenas a cientos de bilinks
  por proyecto) el escaneo de archivos es suficiente. `range.N` en la cache hace trivial la búsqueda posición → endpoints sin base de datos. SQLite añadiría complejidad de sincronización en entornos distribuidos sin beneficio real.
