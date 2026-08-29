# ADR-0003: bilinker — El formato: captures inmutables y aceptación explícita

**Estado:** Propuesto **Fecha:** 2026-08-28 **Actualizado:** 2026-08-29

**Parte de** [la épica del MVP](../../../../../../.stratum/worklist/1.epic.md), que lleva la motivación de entrega. Los otros dos ADRs de la misma tanda: [ADR-0004](0004-bilinks-en-ref-paralela.md), dónde viven los bilinks, y [ADR-0005](0005-frontera-entre-proyectos.md), la frontera entre proyectos. Éste va primero: los otros dos usan su vocabulario.

---

## Contexto

Este ADR resuelve tres defectos del formato actual, encontrados al preparar el trabajo de la épica. Son anteriores a ella y valen por sí solos.

**(a) Contradicción sobre `hash.N` de un endpoint layer.** `bilink.md` (inv. 6), `reference.md:118`, `consistency.md:23` y `capture.md:169` dicen "copia del `hash.N` estructural adyacente"; `chain.md:47`, `check.md` (§ Endpoint layer, paso 4) y `bilinker/architecture.md:57` —más el diagrama de propagación de las líneas 57-63, que lo repite— dicen "SHA-256 del archivo `.bilink` adyacente". Gana `bilink.md`: lo confirman el disco —el `hash.1` del nodo spec de `b95021d2` es `838ea0a4…`, el `hash.1` estructural del nodo impl, no el SHA del archivo, `9211a4f3…`— y el código, donde `check.rs:433` usa `adj_bl.structural_hash()`. El endpoint repo sigue la misma regla (Decisión 4), así que la lectura del hash-del-archivo queda descartada en todos los casos.

**(b) `check` ensucia archivos versionados sin cambio semántico.** Al escribir este ADR, `git status` sobre `accreta` mostraba 16 `.bilink` modificados; el diff completo de cada uno era una línea de `resolved_at`.

**(c) Campos y estados especificados que la implementación descarta.** `KEYS` en `bilink.rs` no incluye `kind` ni `name.N` — se borran al reescribir, y ningún archivo real del corpus los usa. Los endpoints de tipo bilink no existen en el enum `LinkEndpoint`. Y `UNREACHABLE` está declarado en una tabla de `bilink.md` § "Endpoint bilink" y en prosa en `sublayer-config.md:70`, pero **no existe en el código**: no hay variante `Unreachable` en `EndpointState`. Además está sobrecargado: `bilink.md` lo usa para un `.bilink` ausente y `sublayer-config.md` para una subcapa no clonada, que son escalas distintas con arreglos distintos — el desdoblamiento vive en [ADR-0005](0005-frontera-entre-proyectos.md).

### Principios que la decisión respeta

Aridad fija en dos (`bilink.md` inv. 2, commit `dfd4a26`); un capture describe ubicación, nunca aceptación, y no contiene hashes (`capture.md` inv. 1); `apply` corrige ubicación y `accept` fija contenido (`capture.md`); sólo git y tree-sitter; derivados regenerables y nunca fuente de verdad (`index.md` inv. 1); nada difuso cierra solo (`check.md`); y **la aceptación es un acto humano deliberado, que va a ser gobernable por consenso** (`integration/bilinker.md`).

---

## Decisión

### 1. Captures inmutables, direccionados por contenido

El nombre de un capture pasa a ser el hash de su propio contenido, y su contenido es **sólo la ubicación**. Los dos archivos pasan a **YAML**, con los tipos definidos en Rust (Decisión 2):

```
.bilink/
  <uuid>.yaml              ← la relación
  capture/<id>.yaml        ← la ubicación, inmutable; <id> = H(file, query, offset)
  cache/state              ← state · range · commit · todo lo derivable
  version                  ← la versión de formato
```

**La extensión es `.yaml` y nada más.** El tipo lo dice la carpeta que los contiene, así que repetirlo en el nombre sería redundante. Es lo contrario de un ítem de worklist, que es `<id>.<tipo>.md` porque las épicas, las user stories y las tasks comparten directorio y ahí el nombre es lo único que las distingue.

```yaml
# .bilink/<uuid>.yaml
endpoint:
  0:
    link: capture 67ba7217-e033-4051-becd-4921b55a7872     # dónde está ahora · apply
    accepted:                                              # qué se bendijo   · accept
      link: capture 67ba7217-e033-4051-becd-4921b55a7872
      hash: c00e07602bd560755096b57df1ddb9ed49d816fb8af58a4ec9cde82f21f38db3
      hash_ast: 1b9e44a2f0c8d3e7a5b1c9d4e2f6a8b0c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3
  1:
    link: path >impl
```

```yaml
# .bilink/capture/<id>.yaml
file: subsystems/bilinker/commands/check.md
query: |-
  (section (atx_heading (inline) @n0 (#eq? @n0 "Especificación: comando `bilinker check`"))
    (section (atx_heading (inline) @n1 (#eq? @n1 "Estados — endpoints layer"))) @target)
offset: 3226~5109
```

> **`apply` escribe `link`. `accept` escribe `accepted`. `check` no escribe nada en el bilink.**

Un campo cada uno, y la frontera deja de ser una convención de nombres para ser estructura. Y **la invariante de aceptación deja de enunciarse**: hoy hay que decir que los campos aceptados están presentes juntos o ausentes juntos; con el bloque, `accepted` está o no está. `PENDING` es su ausencia, literalmente. Se verificó: `accepted` sin `hash` es rechazado, y un `accepted.hash` suelto afuera del bloque también.

**El tipo de endpoint pasa a ser explícito.** Hoy `bilink.md` dice que *"el tipo se infiere del valor de `link.N` — no hay prefijo de tipo"*, y el endpoint layer es el fallback. Con prefijo son cinco, más una palabra suelta:

| Prefijo | El resto es |
|---|---|
| `capture <uuid>` | un id de capture de esta capa |
| `path <stratum-path>` | un path Stratum |
| `task <id>` | un id de worklist |
| `repo <alias>` | un alias de repo ajeno ([ADR-0005](0005-frontera-entre-proyectos.md)) |
| `bilink <uuid>` | otro bilink (`proposals/bilink-endpoint.md`) |
| `abstract` | nada — no lleva valor |

El prefijo no nombra el destino sino **en qué lenguaje está el resto**, que es lo que el parser necesita saber. `path` sale de Stratum, cuya spec se titula *"lenguaje de paths Stratum"* y cuya regla raíz es `stratum-path`; `layer` habría afirmado de más, porque un stratum-path también cruza a sub-proyectos —`*/subsystems/lattice`— que `layer-model.md` distingue de las capas internas.

Y elimina una regla: [ADR-0005](0005-frontera-entre-proyectos.md) ya no necesita enunciar una **precedencia del parser**, que existía porque el endpoint layer era el fallback. Sin fallback no hay desempate.

**El `hash` del fragmento no entra en el nombre del capture.** Es la perilla de la que se desprende todo lo demás, y estaba mal puesta.

`hash` y `hash_ast` van como valores separados y no hasheados juntos, porque `RESTYLED` necesita compararlos por separado. `accepted.hash_ast` es opcional: ausente donde no hay gramática, y sin valor discriminante en prosa. Donde no está, `RESTYLED` no existe y todo cambio de texto es `ALTERED` — que en prosa es lo correcto. 

#### Por qué YAML, y no un formato propio

El formato de hoy es `clave: valor` plano con líneas de continuación, comentarios y claves numeradas — inventado, con unas 730 líneas de parser a mano entre `bilink.rs` y `link.rs`. Adoptar un formato existente deja sólo serialización y una definición de tipos.

**JSON queda descartado de entrada:** 21 de los 129 captures del corpus tienen queries multilínea, y escaparlas con `\n` arruina el `git diff`, que en [ADR-0004](0004-bilinks-en-ref-paralela.md) **es la superficie de revisión del producto**.

Entre YAML y TOML se midió sobre el corpus real:

| | Medición |
|---|---|
| queries multilínea | 21 de 129. YAML las toma en bloque sin comillas ni escapes; TOML necesita string literal de comilla triple, porque las queries llevan `"` y pueden llevar `\` en un `#match?` |
| queries con `: ` adentro | **92 de 92** — es la sintaxis de campo de tree-sitter. Obliga a que la query vaya **siempre** en bloque, no sólo cuando es multilínea |
| valores con sigilo inicial | 16 `>impl` en el corpus, más `*` y `@` que el diseño usa. En YAML rompen sin comillas — **y eso lo resuelve el prefijo explícito**: `path >impl` arranca con letra |
| coerción de tipos | 3 commits todo-dígitos hoy. `0123` se lee **octal y devuelve 83**; con SHA-1 de 40 caracteres es cuestión de volumen |

Gana YAML, con dos reglas que el serializador fija de una vez:

> La query va **siempre** en bloque `|-`, nunca `|`. Todo valor escalar se escribe citado.

La primera no es cosmética: `|` conserva un salto final, y como el id del capture es `H(file, query, offset)`, **elegir mal el indicador mueve los 129 ids en silencio**. Es lo que el escenario `serialization-is-stable` verifica. La segunda cierra la coerción.

#### Dos comparaciones, dos hechos

| Comparación | Si difiere |
|---|---|
| `link` vs `accepted.link` | se movió y no está aprobado — `RELOCATED` |
| `hash(fragmento en link)` vs `accepted.hash` | cambió el contenido — `ALTERED` o `RESTYLED` según `accepted.hash_ast` |

`OK` es que coincidan las dos. Sin token compuesto y sin abrir ningún objeto: cada dimensión se compara sola y se nombra sola.

#### El segundo operando se vuelve explícito

`check` compara siempre dos cosas: **un estado del mundo** contra **un conjunto de aceptaciones**. Hoy las dos son implícitas y salen del mismo lugar —el árbol de trabajo y el `.bilink/` de al lado—, y por eso nunca hizo falta nombrarlas. Con las aceptaciones reducidas a campos con nombre por endpoint, dejar elegir de dónde sale el segundo operando cuesta una flag y abre las preguntas que hoy no se pueden hacer:

```
bilinker check [<path>] [--against <fuente>]

<fuente> = un commit de refs/bilink/<branch>   → lo commiteado, contra lo que tengo sin commitear
         = otra rama                            → ¿aprobamos lo mismo que ellos?
         = un directorio                        → cualquier .bilink/, incluido .bilink-migrate-<id>/
```

No es un comando nuevo ni una comparación nueva: es la misma de la tabla de arriba, con el lado derecho traído de otra parte. Tres cosas que hoy no tienen respuesta pasan a tenerla, y todas con la misma máquina:

- **Qué estoy por commitear.** El `diff` del índice propio muestra hashes cambiando, que es ilegible; `--against` el commit de la ref lo dice por endpoint y en el vocabulario de estados.
- **Qué hay en un `.bilink/` con trabajo sin commitear.** Cambiar de rama no llega acá: [ADR-0004](0004-bilinks-en-ref-paralela.md) lo corrige solo, materializando sin preguntar. Donde sí se para es en su única guarda —el árbol difiere del commit que `head` nombra, o sea que hay trabajo que no está en ninguna otra parte—, y ahí `--against` ese commit dice qué difiere por endpoint en vez de dejarlo como un error opaco.
- **Qué difiere entre la carpeta migrada y la de al lado**, una vez que las dos están en el formato nuevo — por ejemplo tras regenerar la migración.

**`--against` no cruza versiones de formato.** El binario linkea un solo parser, así que las dos fuentes tienen que ser de la misma versión. Comparar el antes contra el después de una migración no lo puede hacer `check`: lo hace **la migración**, que es el único componente que depende de los dos formatos. Pedirle a `check` que lea el formato viejo sería volver a cargar toda la historia en el camino de lectura, que es justo lo que `migration.md` descarta.

**Con `--against`, `check` no escribe la cache.** Está contestando una pregunta, no verificando el estado de la rama, y `cache/state` describe la rama. Es la misma línea que separa a `--dry-run` de un comando real.

#### Delta contra el formato de hoy

| Hoy | Nuevo | Qué pasó |
|---|---|---|
| `link.0` / `link.1` | `endpoint.0.link` / `endpoint.1.link` | el sufijo `.N` pasa a ser la clave del endpoint |
| — | `accepted.link` | **nuevo** — la ubicación bendecida |
| `hash.N` | `accepted.hash` | mismo significado, adentro del bloque |
| `hash_ast.N` | `accepted.hash_ast` | ídem |
| `commit.N` | → `cache/state` | pasa a derivado (Decisión 3) |
| `state.N` | → `cache/state` | pasa a derivado (Decisión 3) |
| `resolved_at` | — | eliminado (Decisión 3) |
| `kind` · `name.N` | `kind` · `name` (en el endpoint) | dejan de descartarse; `name` deja de ser numerado |

Un campo nuevo, dos renombrados, dos que se mudan, uno que se va — más el cambio de contenedor y el prefijo de tipo. La migración `002` se explica en una línea, y el `.capture` sólo cambia de nombre de archivo (`003`).

#### Por qué la perilla estaba mal puesta

Con el `hash` adentro del id, un capture deja de ser una ubicación y pasa a ser una foto. Entonces `apply` —cuyo trabajo es mover la ubicación sin tocar el contenido— no puede acuñar el capture corregido cuando el texto en la ubicación nueva difiere. Eso es `REANCHORED` (renombrar el anchor cambia el fragmento: `check.md` lo dice con datos, los 60 captures que existen en este proyecto tienen el nombre del anchor adentro del fragmento) y `EXPANDED` (crecer cambia el texto por definición). De ahí salía una restricción aritmética sobre `apply`, y de ahí salía que `accept` tuviera que acuñar captures.

#### Qué se gana

- `apply` conserva su semántica y sus cuatro fixes; se cae la restricción aritmética.
- `accept` **nunca toca un capture**: escribe sólo `accepted` del bilink. La mitad de `capture.md` inv. 7 que dice *"nunca toca un capture"* se refuerza; sólo cambia la lista de campos que nombra.
- Dos bilinks sobre el mismo fragmento comparten **un** capture aunque hayan aceptado distinto. La divergencia vive en `accepted`, que es exactamente donde `capture.md` § "Referencia desde un bilink" siempre dijo que tenía que vivir: *"dos bilinks pueden haber aceptado versiones distintas del mismo fragmento"* sigue siendo expresable, y sin duplicar la descripción de la ubicación.
- `capture.md` inv. 1 (*"Un capture describe ubicación, nunca aceptación. No contiene hashes ni commits."*) sobrevive intacta.
- Dedup por construcción y sin lookup: misma ubicación → mismo id. Se va el escaneo de captures equivalentes que hoy hace `capture_to_file` antes de acuñar un UUID v4 — y con él, la brecha entre `commands/capture.md`, que documenta el UUID, y el código, que ya deduplica.
- El **copy-on-write de `apply` desaparece como concepto**: con captures inmutables no hay mutación en el lugar que proteger, así que se cae la regla de forkear según el tipo de fix y el conteo de referentes que la decide.

El título del ADR aguanta: los captures siguen siendo inmutables y direccionados por contenido — por el contenido **del capture**, que es lo que `capture.md` siempre dijo que un capture era.

#### `accepted.link` — la ubicación también se bendice

Sin este campo, `apply` repunta `link.N` y el endpoint vuelve a `OK` solo, porque el hash no cambió: **un movimiento se cierra sin que nadie lo apruebe**. Con él, `apply` deja la descripción cierta pero el endpoint no-OK hasta que un humano acepte.

> **`apply` describe, `accept` bendice** — y bendecir incluye la ubicación.

Efecto colateral útil: `accepted.link` mantiene vivo el capture que describe dónde estaba el fragmento **cuando se aceptó**, que es lo que hace que `git show <commit>:<file>` siga funcionando después de un rename. Hoy eso se degrada en silencio.

Costo: un rename masivo pasa a necesitar `apply -y` **y** `accept .`, dos comandos en vez de uno. `accept` a secas bendice **las dos dimensiones** —lo que la gente va a querer casi siempre— y `--place` / `--content` restringen, para aprobar un rename sin aprobar un cambio semántico o al revés. El commit registra cuál fue.

| Endpoint | `accepted.link` |
|---|---|
| estructural | id de un capture de su propia capa — resoluble localmente |
| layer · repo | **copia** del valor del vecino, opaca, no resoluble localmente |
| `task` | ausente — la ubicación es el id de la tarea y no se puede mover |
| `abstract` | ausente — no hay nada que bendecir |

Las dos primeras filas son la misma regla que hoy rige `hash.N` (*"copia del valor estructural del vecino"*), ahora con dos valores en vez de uno — así que el endpoint layer y el repo siguen siendo el mismo mecanismo, sin excepción. Es lo que obliga a **reformular `capture.md` inv. 4**: hoy dice que un bilink sólo referencia captures de su propia capa, y ahora un campo puede contener el id de un capture ajeno que nunca se resuelve localmente.

**La invariante de aceptación es el tipo.** El bloque `accepted` está o no está, y si está lleva `hash` obligatorio. Lo que queda por decir son las salvedades por tipo de endpoint, que la tabla de arriba fija: `accepted.link` está ausente para `task` —la ubicación es el id de la tarea y no se puede mover— y para `abstract` no hay `accepted` en absoluto. Reemplaza a `bilink.md` inv. 4, que hoy dice lo mismo sobre `hash.N` y `commit.N` y hay que verificarlo a mano.

#### `RELOCATED` — en qué se diferencia de `MOVED`, y por qué sale con 1

Están en **tablas distintas y en momentos distintos de la misma historia**:

| | `MOVED` | `RELOCATED` |
|---|---|---|
| Tabla | resolución — del **capture** | aceptación — del **endpoint** |
| Pregunta | ¿dónde está el fragmento? | ¿está aprobada la ubicación donde está? |
| Condición | el archivo cambió de path (git rename ≥ 50%) | `link ≠ accepted.link`, con el hash igual |
| Cuándo aparece | en el `check`, **antes** de comparar nada aceptado | **después** de que `apply` corrigió |
| Cómo se sale | `apply` | `accept` |

```
MOVED ──apply──> RELOCATED ──accept──> OK
```

`MOVED` dice *qué* pasó; `RELOCATED` dice *que nadie lo aprobó todavía*. Y `RELOCATED` es uno solo para los tres caminos que llegan ahí: `MOVED` (cambió `file`), `DISPLACED` (cambió `offset`) y `REANCHORED` (cambió `query`). El porqué vive en el capture y en el diff entre los dos captures; el estado de aceptación necesita un solo bit.

**Exit 1.** Hoy `MOVED`/`DISPLACED` salen con 0 (ADR-0002 § "Salida de `bilinker check`"), y ahí tiene sentido: son estados que `apply` cierra sin decisión humana. `RELOCATED` es lo contrario — existe precisamente porque decidimos que **los movimientos se aprueban**. Si saliera con 0, nada obliga nunca a aprobarlos: se acumulan en silencio y `accepted.link` queda decorativo. El código de salida es lo único que convierte "pide acción humana" en algo que un CI o un hook puede hacer cumplir. Si alguna vez molesta que un rename ponga el CI en rojo, la salida es un **exit 2** propio —*"nada está roto, pero hay aprobación pendiente"*— que deja al consumidor elegir; cuesta un valor más en el contrato de salida y por ahora no hace falta.

#### El repunte masivo deja de ser peligroso

Hoy `apply` forkea un capture compartido para no resolverle la corrección a los demás referentes por decreto. Con `accepted.link` esa protección se muda de lugar: **repuntar ya no es aceptar**. `apply` puede repuntar los `link.N` de todos los referentes de un rename —un hecho objetivo sobre el archivo— y cada bilink queda igualmente en `RELOCATED` hasta que su dueño lo apruebe. La decisión sigue siendo una por bilink; lo que se comparte es la descripción, que es lo que el capture siempre fue.

#### `prune` es mark & sweep, no refcount

Las raíces son **todos los `link.N` y todos los `accepted.link`** de los `.bilink` de la capa; se barre lo no alcanzable. `capture.md` define hoy un capture huérfano como el que "ningún `.bilink` referencia", y hoy la única referencia posible es `link.N`; con la raíz nueva sin agregar, `prune` borraría justamente los captures que sostienen la arqueología de lo aceptado.

El índice **acelera** el marcado; no lo autoriza — es un derivado que puede estar viejo (`index.md` inv. 1), y la asimetría es fatal: si sobrestima queda basura, si subestima borra un objeto vivo. Por la misma razón no sirve un contador de referencias: se desincroniza con cualquier escritura que no pase por el índice (`checkout` de otra rama, merge, `revert`, un `apply` que crashea a mitad), que es la razón por la que git usa mark & sweep y no refcounts.

**Y `prune` saca del árbol, nunca de la historia:** lo barrido sigue alcanzable en `refs/bilink/<branch>~n`, que es lo que `get --diff` recorre hacia atrás y lo que el consumidor remoto necesita para ubicar el commit del proveedor. Recuperar un objeto barrido es un `git show`. Nunca corre solo.

### 2. Los tipos viven en Rust, y el esquema sale de ahí

Adoptar YAML deja fuera el parser de contenedor, pero no la definición de qué es un bilink. Ésa va en tipos Rust con `serde` y `schemars`, y el esquema JSON publicable se genera de ellos.

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BiLink { pub endpoint: Endpoints }

/// La aridad es fija en dos, y son distinguibles: `accept <uuid>.0`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Endpoints {
    #[serde(rename = "0")] pub zero: Endpoint,
    #[serde(rename = "1")] pub one:  Endpoint,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub link: Link,
    /// Ausente = PENDING. La invariante de aceptación es este Option.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub accepted: Option<Accepted>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Accepted {
    /// Ausente para `task` y `abstract`.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub link: Option<Link>,
    pub hash: Sha256,
    /// Sólo donde hay gramática tree-sitter.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub hash_ast: Option<Sha256>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(regex(pattern = r"^(capture |path |task |repo |bilink ).+$|^abstract$"))]
pub struct Link(String);
```

**Dos invariantes dejan de ser prosa y pasan a ser el tipo.** Se verificó con `serde_yaml`:

```
dos endpoints                    ACEPTA
tres endpoints                   RECHAZA  — unknown field `2`, expected `0` or `1`
falta el 1                       RECHAZA  — missing field `1`
accepted sin hash                RECHAZA  — endpoint.0.accepted: missing field `hash`
hash suelto fuera de accepted    RECHAZA  — unknown field `hash_accepted`, expected `link` or `accepted`
```

La aridad fija —`bilink.md` inv. 2, la invariante que la spec más defiende— deja de ser algo que hay que verificar y pasa a ser algo que no se puede escribir. Y `deny_unknown_fields` es la lista blanca `KEYS` de hoy **pero ruidosa**: hoy un campo desconocido se descarta en silencio, que es cómo un binario viejo vaciaría las aceptaciones.

También se verificó que las claves enteras de YAML —`0:` y `1:` sin comillas— matchean por `rename` y **no por posición**: una struct con los campos declarados al revés dio el mismo resultado. Así que no hacen falta comillas en las claves.

Lo que el tipo **no** hace: destructurar el prefijo de `link`. Eso sigue en código — pero con el prefijo explícito es partir en el primer espacio y matchear seis palabras, no las 354 líneas de precedencia de `link.rs`.

#### `.bilink/version` — la versión de formato

Un archivo por carpeta con la versión del formato de esos archivos. **Commiteado**, a diferencia de `cache/`, `index/` y `head`: describe archivos versionados y viaja con ellos.

No es lo mismo que el ledger de migraciones, y las dos cosas hacen falta:

| | Qué responde | Granularidad |
|---|---|---|
| `.accreta/migrations` | qué pasos del proceso corrieron acá | por repo |
| `.bilink/version` | qué son estos archivos | por carpeta |

Coinciden casi siempre, y **divergen justo donde importa**:

- **Un cambio aditivo no lleva migración pero sí cambia el formato.** [ADR-0005](0005-frontera-entre-proyectos.md) agrega los endpoints `repo` y `abstract` sin migración —son aditivos— pero un parser viejo lee `abstract` como un path y no falla. El ledger no puede expresar eso: no hubo migración que registrar.
- **La frontera.** El consumidor trae la ref del proveedor con sparse-checkout de `.bilink/`; llegar al ledger ajeno exigiría sumarlo al sparse y **parsear los ids de migración internos de otro proyecto** para deducir si entiende sus archivos. La versión contesta eso directamente, y es lo único que necesita saber del proceso interno del proveedor: nada.

**La versión no se declara a mano: se verifica.** El esquema generado lleva su versión, y un test afirma que su hash corresponde a la registrada. Cambiar el esquema sin subir la versión falla — que es el modo de falla que un campo declarado no cubre.


### 3. Lo derivable sale del formato

El criterio es uno solo: **si un valor se puede reconstruir de los demás, no va en el bilink**. Lo que queda versionado es lo que nadie puede recalcular — la declaración (`link.N`) y la decisión (`_accepted`). Todo lo demás vive en `cache/state`, que no está en git.

Ningún archivo de bilinker se escribe a mano: todos salen de un comando.

**`resolved_at` desaparece.** No se muda a la cache: se va del formato. Tiene un solo uso funcional, repetido en tres lugares (`consistency.md:153` y `:176`, `check.md:194`): ser el baseline de `git log --since=<resolved_at>` para atribuir un cambio a un commit. Y ese uso **está dominado por `commit`**. Un timestamp es un mal baseline para arqueología en git —las fechas se desordenan con rebases, cherry-picks y skew de reloj— mientras que `git log <commit>..HEAD -- <file>` recorre por ancestría y es exacto. El propio `check.md` ya usa `commit.N` para su fast-path, así que hoy el mismo comando tiene dos baselines para la misma pregunta y usa el bueno para una cosa y el malo para la otra. `lattice/integration/bilinker.md:55` también declara que el baseline es `commit.N`. No es un campo que sobra: es una segunda respuesta, peor, a una pregunta ya contestada. Se caen con él las invariantes **12 y 13** de `bilink.md`.

**`kind` y `name.N` se quedan, y se implementan.** Hoy están especificados pero no implementados —`KEYS` los descarta al reescribir, así que un `apply` borra en silencio lo que alguien hubiera puesto—, y la regla de la Decisión 5 mandaría moverlos a `proposals/`. Pero esa vara es *implementarlo o moverlo*, y acá implementarlo son tres claves en `KEYS`, preservarlas al escribir, y un `--kind` en `chain new` para no depender de una edición a mano. Son **inertes** —`bilink.md` inv. 16 ya dice que no afectan `hash.N` ni `state.N`— y encajan sin fricción en el formato nuevo: son campos de declaración, al lado de `link.N` y con el mismo escritor. Lo que sí se mueve es el **endpoint de tipo bilink**, cuyo costo de implementación es un tipo de endpoint entero; y con él queda diferido `governs`, único valor definido de `kind`, que sin ese endpoint no se puede expresar. En un bilink abstracto los dos campos quedan simplemente ausentes.

**El sufijo `.N` sobrevive donde el dato es de una punta.** Los tres campos de aceptación lo llevan, y `link.N` con ellos, porque cada endpoint se acepta en su propio momento y su propio repo. `state.N` lo conserva en la cache por la misma razón: `check` devuelve una tupla justamente porque un extremo puede estar `OK` y el otro `ALTERED`. Lo pierden `range` y `state`, que son del capture, donde hay un único fragmento y no hay qué numerar.

#### `commit` es el commit del contenido, y es derivado

`commit` es **el commit en el que el fragmento quedó con el contenido aceptado**, no el HEAD de quien acepta — que es lo que hace hoy `accept.md`, paso 3. Con esa definición es **derivable de `(accepted.link, accepted.hash)`**, así que por los criterios de la Decisión 5 no pertenece al bilink: va a `cache/state`.

- **Cómo se calcula:** `git log -L <start>,<end>:<file>` (nativo, offline), o un walk acotado hacia atrás resolviendo la query hasta que el hash del fragmento cambie. Se calcula en `accept` —una vez, con todo el contexto a mano— y se escribe en la cache. Un derivado que escribe `accept` en vez de `check` es raro pero legítimo: sigue siendo reconstruible, que es lo único que define un derivado.
- **Arqueología:** `commit..HEAD` excluye `commit`, así que la ventana queda exactamente en "lo que pasó después de que el contenido aceptado quedó establecido". Correcto sin ajustes.
- **Fast-path:** `git diff --name-only <commit> -- <file>` sigue valiendo, con cache tibia; con cache fría se corre el algoritmo completo, que es correcto y más lento.
- **Consecuencia dura:** ese commit **no existe si el fragmento no está commiteado**. `accept` tiene que fallar sobre un archivo sucio. Deja de ser recomendación y pasa a ser requisito, y es lo que garantiza que el snapshot de la ref sea verdadero (Decisión 6).

Como `commit` no es propiedad de la ubicación, tampoco entra en el id del capture: la procedencia de una decisión no es parte de dónde está un fragmento. Es `capture.md` inv. 1 dicha por el otro lado.

Consecuencia: el texto aceptado se recupera con `git show <commit>:<file>` dentro de un repo. Cruzando la frontera **no se recupera por defecto**, porque el clon del proveedor es superficial. Lo que se pierde ahí es exactamente lo que necesita el texto viejo y no sólo su hash: `EXPANDED`, `DISPLACED` y `REANCHORED` no se distinguen, y todo cambio de contenido se reporta como `ALTERED`. La dimensión de ubicación **no** se degrada: se decide comparando dos ids copiados, sin abrir nada del clon. No es un límite del modelo sino del clon, y se levanta a pedido (Decisión 4, § "Profundidad a pedido").

#### `cache/state` — dos clases de derivado

La cache no está en git, así que estar fría es un estado **normal**: clon fresco, cambio de rama, después del corte, `rm -rf .bilink/cache/`, u otra máquina. Adentro conviven dos clases con garantías distintas, y conviene no tratarlas como una sola:

| | Qué es | Cache fría significa | Quién la escribe |
|---|---|---|---|
| `state` · `state.N` · `range` | resultado de la última verificación | **no disponible** — hay que correr `check` | `check` |
| `commit` | memo de un walk caro | **más lento**, nunca no disponible: se re-deriva a demanda | `accept`; re-derivable por quien lo necesite |

Por eso poner `commit` en la cache no bloquea a nadie. Donde hace falta —recuperar el texto aceptado para `EXPANDED`/`DISPLACED`/`REANCHORED`, y la atribución de "Fuente del cambio"— sólo corre sobre endpoints **ya no-OK**, así que el costo está acotado por lo que está roto y no por el total. Si la derivación falla del todo (el contenido aceptado no existe en la historia de esta rama), degrada adonde ya degrada hoy: `check.md` § "Recuperar el texto aceptado" descarta el texto y esas detecciones no corren.

`cache/state` es **un archivo por capa**, igual que `index/index`: reescritura atómica, menos inodos, cero conflictos de merge. `index.md` § "Git" ya sienta el precedente de un derivado dentro de `.bilink/` que puede estar gitignoreado.

**Y registra a qué commit de `refs/bilink/<branch>` corresponde.** Con [ADR-0004](0004-bilinks-en-ref-paralela.md) el árbol de trabajo cambia de rama y `.bilink/` tiene que cambiar con él; una cache por capa sin discriminar rama devolvería estados de otra rama en silencio. Con el commit anotado, la cache se invalida sola si no coincide: se auto-verifica, no multiplica archivos, y no necesita saber nada de nombres de rama. Es la mitad derivada de un par: § "Cambiar de rama" pone la otra, `.bilink/head`, que dice a qué rama y a qué commit corresponde el `.bilink/` del árbol. La cache protege al derivado; `head` protege a la fuente, y por eso `head` no puede vivir acá adentro.

### 4. Migración

`concepts/migration.md` ya define la maquinaria: ids ordenados, ledger por repo en `.accreta/migrations`, runner idempotente, `--dry-run` que no escribe, y la regla de que una migración se marca aplicada sólo cuando corrió sobre **todas** las capas del repo. Las dos nuevas se registran en `commands/migrate.md` junto a `bilinker-001-capture-split`.

**El orden importa y no es el obvio.** La partición va primero: mientras `range`, `state` y `resolved_at` sigan dentro del `.capture`, no se le puede calcular un id estable.

**`bilinker-002-file-partition`** — reescribe cada `.bilink` al formato nuevo: de `clave: valor` plano a YAML, con los endpoints bajo `endpoint.0`/`endpoint.1` y el tipo de cada `link` explícito (`path`, `repo`, …). `hash.N` → `accepted.hash` y `hash_ast.N` → `accepted.hash_ast`; `commit.N` y `state.N` van a `cache/state`, junto con el `range` y el `state` que salen del `.capture`; `resolved_at` **se descarta** en los dos archivos: no se muda, desaparece del formato. `kind` y `name.N` se preservan tal cual —la migración es el momento en que dejan de perderse— y `name.N` pasa a ser `name` adentro de su endpoint. Y se escribe `.bilink/version`. Y `accepted.link` se siembra copiando `link.N` donde `hash.N` está presente. Es exacto donde `state.N` es `OK`: en el formato viejo un endpoint `OK` es uno cuyo contenido actual coincide con el aceptado en la ubicación que `link.N` describe, así que esa ubicación *es* la bendecida. Donde el endpoint está no-OK es la única lectura disponible —el formato viejo no distingue drift de ubicación de drift de contenido— y es la que preserva la invariante de aceptación sin poner los 158 bilinks del ecosistema en `RELOCATED` de golpe ni degradarlos a `PENDING`, que borraría el inventario de trabajo que la Decisión 6 necesita. En un endpoint `PENDING` `accepted` quedan ausentes y sólo sobrevive `link.N`. Todo es copia y renombre —`state.N` se lee del archivo, no se recalcula—: no se resuelve ninguna query ni se consulta git, como exige `migration.md` inv. 5.

**`bilinker-003-immutable-captures`** — reescribe cada `.capture` como `capture/<H(file, query, offset)>.yaml` y repunta cada `link` y cada `accepted.link`. No tiene fan-out: como el id no depende del hash, dos bilinks que aceptaron contenidos distintos del mismo fragmento siguen compartiendo un capture, y la divergencia queda en sus `_accepted`. Dos captures con la misma ubicación colapsan en uno, que es la dedup por construcción. Y un endpoint en `PENDING` no plantea ningún problema: el id nunca dependió de `hash.N`, así que se computa igual que para cualquier otro.

**No hace falta migración para las decisiones 3 y 4.** Los endpoints `abstract` y repo son aditivos: ningún archivo existente los usa, y todos siguen siendo válidos. La frontera se puede adoptar bilink por bilink, sin tocar nada de lo que ya está.

**Mover los bilinks a una ref tampoco es una migración, y no puede serlo.** Mover los bilinks a `refs/bilink/<branch>` no transforma ningún archivo: los deja idénticos y cambia dónde viven. Y `migration.md` inv. 5 prohíbe que una migración consulte git, que es todo lo que esta operación hace. Es un paso único y aparte, por repo.

#### El problema real es de bootstrap

La herramienta que cambia de formato es la que se usa para cambiarlo, y las specs que describen el formato están linkeadas con bilinks al código que lo implementa. Durante la transición conviven tres cosas en tres repos: specs viejas y nuevas, binario viejo y nuevo, y bilinks en los dos formatos. No alcanza con ordenar las migraciones: hace falta que las dos versiones **coexistan** un rato.

**Coexistencia por path.** Los dos formatos no pueden ocupar `.bilink/` a la vez, así que la migración escribe en un path transitorio y deja `.bilink/` intacto. El binario viejo sigue funcionando contra `.bilink/` sin enterarse; el nuevo se ejerce contra el path nuevo con datos reales antes de que nada sea irreversible. Es lo que hace barata la validación: los dos binarios corriendo en el mismo instante sobre el mismo repo, cada uno contra la carpeta que entiende. Y ésa es la única forma de contrastar los dos formatos desde afuera — `check --against` no cruza versiones, y quien sí puede compararlos es la migración, que depende de los dos. Generar el formato nuevo en una rama y mergearla no lo daría — habría que switchear de rama para pasar de un binario al otro, y se pierde la comparación lado a lado.

**El path lleva el id de la migración: `.bilink-migrate-<id>`.** Sin él, dos migraciones en vuelo colisionan, y una carpeta abandonada de un intento anterior es indistinguible de una en curso. Con él, cada paso es inspeccionable y regenerable por separado, y la secuencia se encadena sola:

```
.bilink/  →  .bilink-migrate-002-file-partition/  →  .bilink-migrate-003-immutable-captures/
```

El corte toma la última. El prefijo `bilinker-` del id se omite, que dentro del directorio de bilinker es redundante.

**Es un derivado, no un espacio de trabajo.** No se edita a mano: si se lo edita, deja de poder regenerarse, que es lo único que lo vuelve seguro. Y tiene que poder regenerarse, porque si entre la generación y el corte alguien acepta algo con el binario viejo en `.bilink/`, la copia migrada queda vieja y el corte se comería esa aceptación. Queda en la misma categoría que `cache/state` y `index/`.

**La idempotencia tiene dos regímenes, y hay que decir en cuál está cada cosa.** **Antes del corte**, `migrate` siempre regenera: la carpeta transitoria es un derivado, y regenerar es exactamente lo que recupera un `accept` hecho con el binario viejo. La regla operativa es **regenerar justo antes de cortar**. **Después del corte**, el ledger la vuelve no-op, que es lo que `migration.md` inv. 3 exige.

**`.git/info/exclude` recibe `.bilink-migrate-*` al empezar la migración**, no en el corte: es temporal, se borra al terminar, y esas carpetas nunca se commitean.

**La entrada en el ledger va en el corte, no en la generación.** Si `.accreta/migrations` se escribiera al generar, el repo quedaría marcado como migrado mientras sigue corriendo el formato viejo. Es el mismo principio que `migration.md` ya aplica al exigir que una migración se marque recién cuando corrió sobre todas las capas del repo: se registra cuando el estado es verdadero, no cuando el trabajo empezó.

**No hace falta migración para los endpoints `abstract` y repo** ([ADR-0005](0005-frontera-entre-proyectos.md)): son aditivos, ningún archivo existente los usa, y todos los actuales siguen siendo válidos.

**Mover los bilinks a una ref** ([ADR-0004](0004-bilinks-en-ref-paralela.md)) tampoco es una migración, y no puede serlo: no transforma ningún archivo —los deja idénticos y cambia dónde viven— y `migration.md` inv. 5 prohíbe que una migración consulte git, que es todo lo que esa operación hace. El corte va en ese ADR.

### 5. Lo especificado y no implementado se mueve, no se borra

> **Lo que está especificado y no implementado no se borra: se mueve a `subsystems/bilinker/proposals/`.**

`proposals/` es un directorio nuevo, hermano de `concepts/`, `commands/` y `scenarios/`. La spec pasa a describir lo que existe; la idea sobrevive con su motivación intacta y vuelve cuando alguien la implemente. Vale para todo el defecto (c), y resuelve la asimetría que quedaría si no —por qué `task` sobrevive y el endpoint de tipo bilink no— sin tener que justificarla: los dos se miden con la misma vara.

Va a `proposals/` el **endpoint de tipo bilink**, con sus dos casos de uso: `kind: governs`, y el bilink de tarea, que lo necesita para colgar una tarea de *la relación* y no de uno de sus extremos. Se lleva su motivación, no sólo su definición, más la lista de lo que depende de él — specs de impact, worklist y lattice que pasan a describir algo que todavía no existe, que es exactamente lo que el directorio significa.

**`kind` y `name.N` no se mueven: se implementan** (Decisión 1). La vara es la misma —*implementarlo o moverlo*— y de un lado el costo son tres claves en `KEYS`, del otro un tipo de endpoint entero.

### 6. Los bilinks que queden NO-OK no se aceptan a ciegas

Reescribir las specs de bilinker va a poner en `ALTERED` los 63 bilinks de la raíz de `accreta` que vinculan fragmentos de spec con su implementación, y en `CHAIN_DIRTY` a sus 63 contrapartes del lado impl. Correr `accept .` para dejar el árbol verde sería tirar justamente la información que hace falta: **cada uno de esos estados es un puntero al código que hay que revisar.** El inventario de trabajo del cambio *es* la lista de no-OK, y se cierra endpoint por endpoint, después de que la implementación efectivamente coincida — no antes para acallar el reporte. Es bilinker aplicado a sí mismo, que es la prueba más exigente de si el mecanismo sirve.

---

## Consecuencias

**Invariantes nuevas.** El nombre de un capture es el hash de su ubicación, y un capture es inmutable · `apply` escribe `link.N`, `accept` escribe `accepted`, `check` no escribe nada en el bilink · `accepted` de un endpoint están presentes juntos o ausentes juntos.

**Invariantes enmendadas.** `capture.md` inv. 4 se reformula: un `link.N` sólo referencia captures de su propia capa, pero un `accepted.link` de endpoint layer o repo contiene una copia opaca del id ajeno, que no se resuelve localmente · `bilink.md` inv. 4 pasa a hablar de `accepted` en vez de `hash.N`/`commit.N` · queda la versión de `bilink.md` para el hash del endpoint layer, ahora con dos valores copiados en vez de uno · caen las invariantes 12 y 13 de `bilink.md`, con `resolved_at`.

**Se va.** El copy-on-write de `apply` y la regla de fork por tipo de fix. `resolved_at` del formato. El escaneo de captures equivalentes que hoy hace `capture_to_file`.

**Estados nuevos.** `RELOCATED` — la ubicación cambió y nadie la aprobó, con exit 1.

**Efecto por comando.** `capture` devuelve el id de la ubicación, sin lookup · `check` compara las dos dimensiones contra `accepted`, escribe a `cache/state` y no commitea nada, y gana `--against` para tomar `accepted` de otro lado sin escribir cache · `accept` escribe `accepted`, exige el fragmento commiteado, calcula el `commit` del contenido para la cache y gana `--place` / `--content` · `apply` deja de forkear por tipo de fix, acuña captures sin restricción y repunta `link.N` sin devolver el endpoint a `OK` · `capture prune` pasa a mark & sweep sobre dos clases de raíz · `chain new` gana `--kind` · `index` sin cambios.

**El skill se reescribe en el mismo cambio, no después.** `ia/skills/bilinker/SKILL.md` documenta en detalle todo lo que este ADR cambia, y **se carga solo** cuando alguien trabaja con bilinks: un skill viejo no es documentación desactualizada sino una instrucción activa de crear bilinks en el formato anterior. Dato colateral: `SKILL.md:67` describe bien el `hash.N` de un endpoint layer, o sea que es un cuarto testigo del lado correcto del defecto (a).

**Lattice pierde los rangos y los estados de un clon fresco.** Su nodo canónico es `<layer-root>::<path>#<start>~<end>`, y ese rango sale del `range` del capture; el campo `state` de cada arista sale de `state.N`. Hoy los dos están commiteados, así que un clon basta. Con la Decisión 3 pasan a `cache/state`, fuera de git: hasta que no corra un `check`, lattice no tiene rangos ni estados. No es fatal —`check` es offline y barato— pero es una dependencia nueva que hay que escribir. La otra la agrega [ADR-0004](0004-bilinks-en-ref-paralela.md).

**Trabajo que dispara en las specs.** Sanear los defectos (a), (b) y (c). Reescribir `concepts/capture.md` —cuyo § "Ubicación" dibuja el árbol de `.bilink/`—; crear `concepts/cache.md` y `concepts/accept.md`; crear `proposals/`; ajustar `concepts/bilink.md` (inv. 4, 12 y 13), `reference.md`, `chain.md:47`, `consistency.md` (§ "Fuente del cambio") y `architecture.md` (la línea 57 y el diagrama de 57-63); y los comandos `capture` (que hoy escribe `resolved_at` y `state: RESOLVED` en el capture), `check`, `accept` (que hoy toma el HEAD de quien acepta), `apply` (que hoy forkea por tipo de fix y devuelve `MOVED`/`DISPLACED` a `OK`), `status`, `migrate` y `chain`. En worklist, `concepts/open-bilink.md`, que contiene un `.bilink` completo en formato viejo con `resolved_at`. En la raíz, `concepts/migration.md:26`, que dice "cuatro capas con `.bilink/` repartidas en tres repos" — el mismo recuento viejo que este ADR corrige. En lattice, `integration/bilinker.md` y `concepts/node.md:14`, que siguen diciendo `range.N`, campo inexistente desde el split de captures. La implementación vive en `subsystems/bilinker/.stratum/impl/crates/bilinker/src/`.

---

## Relación con ADR-0002

ADR-0002 sigue vigente en lo estructural —referencias por nodo AST, `@target`, cadenas— pero quedó desactualizado en cuatro puntos: describe `.bilinker.toml` con workspaces, que `configuration.md` ya niega; describe el endpoint estructural como `workspace :: file :: query [:: start~end]`, previo al split de `.capture`; enumera diez estados en el bilink, que hoy están partidos entre capture y bilink; y su § "Salida de `bilinker check`" hace salir con 0 a `MOVED` y `DISPLACED`, que con `RELOCATED` pasan a exigir aprobación humana.

---

## Alternativas descartadas

- **El `hash` del fragmento en el id del capture.** Obliga a `accept` a acuñar captures, deja a `apply` sin poder arreglar `REANCHORED` ni `EXPANDED`, y fuerza un fan-out de captures en la migración `003`.
- **Un almacén de aceptaciones aparte.** El objeto queda vacío: si fuera direccionado por contenido su id sería `H(hash, hash_ast?)`, y como `commit` se fue a la cache, el archivo no tendría nada más que su propio nombre. Un objeto cuyo contenido es su id es un valor, y un valor va en el bilink. Con la ubicación pasa lo mismo: el objeto bendecido ya existe —el `.capture`—, así que bendecirlo es guardar su id. Eso disuelve de una vez el segundo almacén, su barrido, el choque de nombres entre un `<uuid>.accept` y un `accept/<hash>.accept`, y la duplicación de N archivos idénticos que lo motivaba.
- **`<capture>.accept`, donde la existencia del archivo significa "aceptado".** La aceptación tiene sujeto además de objeto: con un solo archivo por fragmento, un bilink hereda la aprobación de otro sin que nadie lo haya mirado, el acto de aceptar se colapsa con el de repuntar, y con la aceptación votable el voto de una relación vale para todas.
- **`H(hash, hash_ast?)` como identidad de la aceptación.** Dos fragmentos distintos con texto idéntico —un método de una línea, un `use`, boilerplate repetido— colapsarían en la misma aceptación. La aceptación es una entrada de tree `(path, blob)`, no un blob suelto: bendecir es decir *"este contenido, acá"*.
- **El id de la aceptación como hash de un commit**, en sus tres lecturas. *El commit de la aceptación* es circular: el archivo va adentro de ese commit, así que su nombre es parte del árbol que el hash cubre. *El HEAD al aceptar* no es único —un `accept .` da N aceptaciones en el mismo HEAD— y es la semántica que la Decisión 2 descarta. *El commit del contenido* —el que la Decisión 2 sí adopta como **valor**— tampoco sirve como **identidad**: no es único, porque un commit establece el contenido de muchos fragmentos, y rompe la convergencia entre ramas.
- **`resolved_at` mudado a la cache** en vez de eliminado. Sería conservar una segunda respuesta, peor, a una pregunta que `commit` ya contesta por ancestría.
- **Refcount en el índice para el `prune`.** Un derivado no puede autorizar un borrado, y un contador se desincroniza con cualquier escritura que no pase por el índice.