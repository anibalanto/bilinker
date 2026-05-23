# ADR-0002: bilinker — Referencias estructurales universales y bidireccionales

**Estado:** Propuesto  
**Fecha:** 2026-05-20

---

## Contexto

Estrato (ex Genia) define capas de conocimiento que van desde documentación hasta
implementación. Actualmente el campo `impl: ref:` usa referencias por número de línea
(`"file::start~end"`), que son frágiles ante cualquier refactor.

Se necesita un sistema de referencias que:

- Sea estable ante reformateos, cambios de indentación y movimientos de código.
- Funcione para cualquier lenguaje estructurado: código (Java, Rust, Python, TypeScript,
  Kotlin...), documentos (Markdown, YAML, TOML, JSON, SQL, GraphQL) y cualquier
  lenguaje con gramática tree-sitter.
- Permita referenciar fragmentos con granularidad arbitraria: una función completa,
  una lambda dentro de un método, un párrafo bajo un heading, un campo YAML.
- Detecte consistencia: si el fragmento referenciado cambia, el link lo sabe.
- Sea bidireccional: dado un nodo, se puede saber quién lo referencia.

---

## Decisión

### 1. bilinker como herramienta independiente

bilinker es una librería y CLI independiente de Acreta, Estrato y expancode.
Es infraestructura — sus consumidores son:

- **Estrato** — reemplaza `impl: ref: "file::start~end"` por referencias bilinker
- **expancode** — usa bilinker para expandir inline el fragmento referenciado
- **Acreta** — usa bilinker para detectar drift entre capas (spec ↔ código ↔ tests)

### 2. Anatomía de una referencia bilinker

Una referencia bilinker tiene cuatro componentes, los últimos dos opcionales:

```
workspace :: file :: query [:: start~end]
```

```
[workspace]          →  dónde buscar y con qué gramática tree-sitter
  [file]             →  qué archivo dentro del workspace
    [query]          →  qué nodo AST dentro del archivo (âncora estructural)
      [start~end]    →  (opcional) sub-fragmento en bytes relativo al nodo
```

**`workspace`** — nombre definido en `.bilinker.toml` que provee el directorio raíz
y el lenguaje (gramática tree-sitter) para parsear los archivos.

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

Cuando bilinker se usa dentro de un proyecto expancode, puede leer los workspaces
de `.expancode.toml` directamente; solo necesita `language` como campo adicional.

**`file`** — ruta relativa a la raíz del workspace. Es parte de la identidad:
distingue clases homónimas en distintos paquetes y aplica igual a código y documentos.

**`query`** — S-expression tree-sitter que identifica el nodo contenedor. Se construye
subiendo en el AST desde la selección hasta el primer ancestro con nombre estable
(función, clase, heading, campo). Ejemplos:

```scheme
; Método en Java
(class_declaration
  name: (identifier) @_ (#eq? @_ "Persona")
  body: (class_body
    (method_declaration
      name: (identifier) @_ (#eq? @_ "vote") @target)))

; Párrafo bajo un heading en Markdown
(section
  (atx_heading (inline) @_ (#eq? @_ "Decisión de arquitectura"))
  (paragraph) @target)

; Campo en YAML
(block_mapping_pair
  key: (flow_node) @_ (#eq? @_ "impl")
  value: (_) @target)
```

**`start~end`** — (opcional) offsets en bytes relativos al inicio del nodo matcheado
por la query. Permite referenciar un sub-fragmento dentro del nodo: una oración dentro
de un párrafo, una expresión dentro de un método.

Si se omite, la referencia apunta al nodo AST completo.

Si se incluye, es un hint auto-corregible: cuando el hash del fragmento se encuentra
en un offset diferente al guardado (texto insertado antes de la selección dentro del
mismo nodo), bilinker actualiza `start~end` en el `.bilink` silenciosamente.

**`hash`** (en cache) — SHA-256 del texto exacto del fragmento seleccionado.
Es la fuente de verdad: si el hash no matchea en ninguna posición dentro del nodo,
el link está genuinamente roto.

### 3. Formato del archivo `.bilink`

Un bilink conecta exactamente dos extremos (`link.0` y `link.1`). Si se necesita
conectar más de dos elementos, se crean múltiples bilinks — un bilink puede referenciar
el `id` de otro bilink como extremo.

Se almacena en archivos `.bilink` dentro de una carpeta `.bilinker/` en la raíz del
proyecto (un archivo por bilink, con un nombre descriptivo).

```
# .bilinker/persona/voting-impl.bilink

id: persona-voting-impl

link.0: java-demo :: persona/Persona.java :: (class_declaration name: (identifier) @_ (#eq? @_ "Persona") (method_declaration name: (identifier) @_ (#eq? @_ "vote") @target))
link.1: specs :: persona/voting.yaml :: (block_mapping_pair key: (flow_node) @_ (#eq? @_ "impl") value: (_) @target)

# --- cache generada por bilinker, no editar a mano ---
hash.0: e9f1a2b3c4d5e6f7
hash.1: a1b2c3d4e5f6a7b8
resolved_at: 2026-05-20T10:00:00Z
```

Para un sub-fragmento dentro de un nodo (e.g. una oración en Markdown):

```
# .bilinker/architecture/decision-ref.bilink

id: architecture-decision-ref

link.0: persona-voting-impl                                                      ← referencia a otro bilink por id
link.1: docs :: architecture.md :: (section (atx_heading (inline) @_ (#eq? @_ "Decisión")) (paragraph) @target) :: 42~87

# --- cache ---
hash.1: c3d4e5f6a7b8c9d0
resolved_at: 2026-05-20T10:00:00Z
```

**Identidad (inmutable, escrita por el usuario o por `bilinker capture`):**
- `id` — nombre estable del bilink
- `link.0` / `link.1` — cada extremo es `workspace :: file :: query [:: start~end]`
  o el `id` de otro bilink.
  El `start~end` es opcional: se omite cuando la referencia es el nodo AST completo,
  se incluye cuando se quiere un sub-fragmento dentro del nodo.

**Cache (generada y actualizada automáticamente por bilinker):**
- `hash.N` — hash SHA-256 del fragmento en la última verificación exitosa.
  Es la fuente de verdad para detectar drift de contenido.
- `resolved_at` — timestamp de la última resolución exitosa.

La ruta absoluta del archivo no se cachea: bilinker la calcula en todo momento
como `workspaces.<ws>.path + file` desde `.bilinker.toml`.

### 4. Semántica de consistencia

Dado un bilink, bilinker puede estar en tres estados por extremo:

| Estado | Condición | Acción |
|---|---|---|
| **OK** | query encuentra el nodo, hash matchea en offset esperado | ninguna |
| **DESPLAZADO** | query encuentra el nodo, hash matchea en offset diferente | actualizar offset silenciosamente |
| **ROTO** | query no encuentra el nodo, o hash no matchea en ninguna posición | alerta al usuario |

El estado DESPLAZADO ocurre cuando se agrega código dentro del nodo contenedor
pero fuera del fragmento referenciado — es un cambio inocuo. bilinker lo resuelve
automáticamente al re-escanear.

El estado ROTO ocurre cuando el fragmento referenciado fue modificado o el nodo
contenedor fue eliminado/renombrado — requiere intervención humana.

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
link.N: java-demo :: persona/Persona.java :: (class_declaration name:#eq?Persona (method_declaration name:#eq?vote @target))

# Selección parcial dentro de un nodo → con start~end
$ bilinker capture docs architecture.md 34:10 34:52
link.N: docs :: architecture.md :: (section (atx_heading (inline) @_ (#eq? @_ "Decisión")) (paragraph) @target) :: 42~87
```

Cuando la selección cae exactamente en los límites de un nodo AST, `start~end` se omite.
Cuando la selección es un sub-fragmento del nodo, `start~end` se incluye automáticamente.

### 6. Subcomando `bilinker check`

Verifica todos los links de un archivo `.bilink` o de un directorio `.bilinker/`:

```bash
bilinker check .bilinker/persona/voting-impl.bilink

OK        link.0 → java-demo :: Persona#vote (lambda)
DESPLAZADO link.1 → specs :: voting.yaml impl  (offset actualizado)
ROTO      link.0 → java-demo :: Reporter#generate_report  (hash no encontrado)
```

Sale con código 1 si hay links ROTOS. Los DESPLAZADOS se auto-corrigen en el archivo
`.bilink`.

### 7. Subcomando `bilinker refs`

Bidireccionalidad: dado un archivo y posición, muestra qué bilinks lo referencian.

```bash
bilinker refs java-app/src/main/java/ar/example/demo/persona/Persona.java:12

.bilinker/persona/voting-impl.bilink  (link.0)
.bilinker/architecture.bilink         (link.0)
```

bilinker mantiene un índice local (SQLite) que registra qué nodos son referenciados
por qué bilinks, actualizado incrementalmente vía `notify` (filesystem watcher) +
tree-sitter `Tree::edit()` para re-parseo eficiente.

### 8. Carpeta `.bilinker/` organizada por capa de Estrato

```
.bilinker/
  persona/
    voting-impl.bilink      # spec ↔ código
    voting-test.bilink      # spec ↔ test
  architecture/
    layers.bilink           # ADR ↔ código
```

Los bilinks viven junto a los specs y docs — no dentro de los archivos referenciados.

### 9. Lenguajes soportados

Cualquier lenguaje con gramática tree-sitter. Los nodos considerados "âncoras
estables" por defecto:

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
- El offset es auto-reparable; solo el hash roto requiere intervención humana.
- Bidireccional por diseño: impacto analysis nativo.
- Independiente de Acreta/Estrato — se puede usar en cualquier proyecto.

**Negativas:**
- Requiere que tree-sitter tenga gramática para el lenguaje del archivo referenciado.
- Las âncoras estables dependen del lenguaje — bilinker necesita conocer qué nodos
  son "estables" para cada gramática.
- El índice SQLite es local; en entornos distribuidos (Acreta P2P) necesita
  sincronización (fuera del scope de este ADR).

---

## Relación con ADR-0001

ADR-0001 define el campo `link:` en expancode usando LSP para resolver símbolos.
bilinker es complementario: donde LSP resuelve símbolos en tiempo real contra un
language server, bilinker registra referencias estructurales persistentes con
consistencia verificable. A largo plazo, `expancode symbol` podría generar
referencias bilinker en lugar de referencias LSP puras.

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
