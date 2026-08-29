# Respuesta a REVIEW-0003

Documento de trabajo sobre los 23 hallazgos de la revisión adversarial de ADR-0003 (`REVIEW-0003.md`). Se saldaron todos; esto es el plan del volcado. Cuando el volcado esté hecho, este archivo se borra.

**Estado:** los 23 hallazgos cerrados, y las 4 decisiones que quedaban abiertas tomadas (§7). **Falta el volcado**, que son tres piezas con riesgo distinto:

1. **El ADR** (`0003-captures-inmutables-y-bilinks-abstractos.md`, repo de impl) — reescritura mayor. La Decisión 1 cambia de raíz (§1), la 3 y la 4 ganan `@` y los estados nuevos (§3, §7), la 5 pierde su tabla de cuatro archivos (§1), la 6 cambia de argumento y gana el orden del corte (§2), la 7 gana el orden y la regla de `proposals/` (§2, §7). Las alternativas descartadas absorben §8.
2. **Los escenarios** (`…-scenarios.yaml`, mismo repo) — el detalle en §6.
3. **Los movimientos a `proposals/`** — toca `subsystems/bilinker/concepts/`, que vive en el **repo raíz**, no en el de impl. Va aparte, y va a poner en no-OK algunos de los 63 bilinks de la raíz.

El formato acordado está en §1; lo descartado y su motivo, en §8 — esa sección existe para que nada de lo que ya se evaluó vuelva a proponerse sin su argumento a la vista.

---

## Tablero

| # | Hallazgo | Estado | Resolución |
|---|---|---|---|
| H1 | El `ALTERED` falso del consumidor remoto se apoya en un check que la Decisión 4 descarta | cerrado | Se reemplaza por la **invariante de fidelidad del snapshot**; absorber pasa a ser precondición de todo commit sobre la ref |
| H2 | La Decisión 6 esconde de la forja el `.accept` que la Decisión 5 quiere revisable | cerrado | La autoría/firma es la del commit de la ref; la superficie de revisión es la de bilinker, no la de la forja |
| H3 | "La Decisión 6 no afecta a lattice" es falso en un clon fresco | cerrado | Redacción: lattice depende de bilinker y dispara fetch + materialización + `check` |
| H4 | La alternativa "el capture guarda el texto" se rechaza con una premisa que la Decisión 4 no cumple | cerrado | Nueva justificación: chunk congelado = copia sin procedencia; el checkout es verificable y navegable |
| H5 | La tabla de absorción atribuye a `apply` un commit que `apply` no produce | cerrado | Se disuelve con H1: absorber deja de ser por comando |
| F1 | `REANCHORED` "hash igual" contradice `check.md`, y rompe la aritmética de `apply` | cerrado | El `hash` del fragmento **sale del id del capture** |
| F2 | "63 bilinks en cuatro capas" | cerrado | 158 archivos · 79 cadenas · 6 capas |
| F3 | `\|hsi` no es sintácticamente inválido como path Stratum | cerrado | Separador `@`; la justificación se reescribe como precedencia del parser |
| F4 | `check.rs:405` | cerrado | Es `check.rs:433` |
| F5 | "`UNREACHABLE` está sólo en prosa" | cerrado | Está en tabla; lo cierto es que no existe en el código |
| C1 | La tabla de la Decisión 5 no lista a `accept` como escritor de `.capture` | cerrado | La tabla queda como está; se corrige la Decisión 1 |
| C2 | La invariante enmendada cita `hash.N`, un campo eliminado | cerrado | Los `_accepted` del bilink (§1); `prune` pasa a mark & sweep |
| C3 | Los endpoints de tipo bilink desaparecen por omisión | cerrado | Se degradan a `proposals/`, junto con `kind` y `name.N` — regla general para todo el defecto (c) |
| C4 | La lista de specs a tocar no cubre lo que dice "Efecto por comando" | cerrado | Lista completa abajo |
| D1 | El corte es el único commit en que el proyecto toca `.bilink/` | cerrado | Orden de 7 pasos abajo |
| D2 | `LAYER_UNREACHABLE` se superpone con `TODO` y `BROKEN` | cerrado | Taxonomía de 5 filas; `UNREACHABLE` a secas desaparece |
| D3 | `commit.N..HEAD` no es exacto si `commit.N` no es ancestro de HEAD | cerrado | Auditar contra la ref, no contra la rama · `bilinker track` adoptado |
| D4 | Absorber no vuelve verdadero el snapshot si se aceptó contenido sin commitear | cerrado | `accept` exige el fragmento commiteado — ahora **forzado** por la definición de `commit` |
| D5 | El índice propio de bilinker cubre todo el árbol | cerrado | `read-tree` del commit absorbido + `update-index` sólo de `.bilink/` |
| D6 | "La ref nunca se mergea de vuelta" no es exigible | cerrado | Dos verificaciones antes de cada commit + `verify-ref` + `repair --unmerge` |
| D7 | La dedup de captures se re-fragmenta | cerrado | Se disuelve con F1 |
| D8 | "Offline" pasa a ser condicional | cerrado | Enmendado |
| D9 | Ambigüedad de "idempotente" | cerrado | Ignorar `.bilink-migrate-*` siempre; dos regímenes explícitos |

---

## 1. El formato, después de F1 · C1 · C2 · D7

Es el cambio de mayor alcance, y sale de una sola perilla: **el `hash` del fragmento no entra en el nombre del capture.** De ahí se desprende todo lo demás.

### El formato

```
capture/<H(file, query, offset)>.capture     ← el único almacén: ubicaciones, inmutables
    file, query, offset

<uuid>.bilink                                ← la relación
    link.0:              capture <id>        dónde está ahora     · apply
    link_accepted.0:     capture <id>        dónde se bendijo     · accept
    hash_accepted.0:     <sha256>            qué se bendijo       · accept
    hash_ast_accepted.0: <sha256>            (opcional)           · accept
    …lo mismo para .1

cache/state                                  ← state · range · commit · todo lo derivable
```

`hash_accepted` y `hash_ast_accepted` van como valores separados y no hasheados juntos, porque `RESTYLED` necesita compararlos por separado. (`hash_ast_accepted.N` se puede acortar a `ast_accepted.N` sin perder nada: `hash_ast` sólo existe como valor aceptado.)

**El sufijo hace legible quién escribe qué**, que era el objetivo de la partición en cuatro archivos de la Decisión 5 — conseguido acá con una convención de nombres:

> `apply` escribe `link.N`. `accept` escribe los `_accepted`. `check` no escribe nada en el bilink.

### Dos comparaciones, dos hechos

| Comparación | Si difiere |
|---|---|
| `link.N` vs `link_accepted.N` | se movió y no está aprobado — ver §3 |
| `hash(fragmento en link.N)` vs `hash_accepted.N` | cambió el contenido — `ALTERED` o `RESTYLED` según `hash_ast_accepted.N` |

`OK` es que coincidan las dos. Sin token compuesto y sin abrir ningún objeto: cada dimensión se compara sola y se nombra sola.

### Delta contra el formato de hoy

| Hoy | Nuevo | Qué pasó |
|---|---|---|
| `link.N` | `link.N` | igual |
| — | `link_accepted.N` | **nuevo** — la ubicación bendecida |
| `hash.N` | `hash_accepted.N` | renombrado; mismo significado |
| `hash_ast.N` | `hash_ast_accepted.N` | renombrado |
| `commit.N` | → `cache/state` | pasa a derivado |
| `state.N` | → `cache/state` | pasa a derivado |
| `resolved_at` | — | eliminado |
| `kind` · `name.N` | — | eliminados |

Un campo nuevo, dos renombrados, dos que se mudan, tres que se van. La migración `002` se explica en una línea, y el `.capture` sólo cambia de nombre de archivo (`003`).

### Por qué no hay almacén de aceptaciones

Se consideró y se descartó por vacío. Si el objeto de aceptación fuera direccionado por contenido, su id sería `H(hash, hash_ast?)` — pero entonces `hash` y `hash_ast` están en el id y `commit` se fue a `cache/`, así que **el archivo no tiene nada más que su propio nombre**. Un objeto cuyo contenido es su id es un valor, y un valor va en el bilink. Con la ubicación pasa lo mismo: el objeto bendecido ya existe —el `.capture`—, así que bendecirlo es guardar su id.

Eso disuelve de una vez: el segundo almacén y su barrido, el choque de nombres entre un `<uuid>.accept` y un `accept/<hash>.accept`, y la duplicación de N archivos idénticos que motivaba el almacén — no hay archivos que duplicar.

Descartado también, por dirimente: **direccionar la aceptación sólo por el capture** (`<capture>.accept`, donde la existencia del archivo significa "aceptado"). La aceptación tiene sujeto además de objeto: con un solo archivo por fragmento, un bilink hereda la aprobación de otro sin que nadie lo haya mirado, el acto de aceptar se colapsa con el de repuntar, y con la aceptación votable el voto de una relación vale para todas.

Y descartado **`H(hash, hash_ast?)` como identidad de la aceptación**: dos fragmentos distintos con texto idéntico —un método de una línea, un `use`, boilerplate repetido— colapsarían en la misma aceptación. La aceptación es una entrada de tree `(path, blob)`, no un blob suelto: bendecir es decir *"este contenido, acá"*.

### Por qué la perilla estaba mal puesta

Con el `hash` adentro del id, un capture deja de ser una ubicación y pasa a ser una foto. Entonces `apply` —cuyo trabajo es mover la ubicación sin tocar el contenido— no puede acuñar el capture corregido cuando el texto en la ubicación nueva difiere. Eso es `REANCHORED` (renombrar el anchor cambia el fragmento: `check.md` lo dice con datos, 60 de 60 captures de este proyecto) y `EXPANDED` (crecer cambia el texto por definición). De ahí salía la restricción aritmética, y de ahí salía que `accept` tuviera que acuñar captures.

### Qué se gana

- `apply` conserva su semántica y sus cuatro fixes; se cae la restricción aritmética.
- `accept` **nunca toca un capture**: escribe sólo los `_accepted` del bilink.
- Dos bilinks sobre el mismo fragmento comparten **un** capture aunque hayan aceptado distinto. La divergencia vive en los `_accepted`. Se disuelve D7 y desaparece el fan-out de la migración `003`.
- `capture.md` inv. 1 (*"un capture describe ubicación, nunca aceptación; no contiene hashes"*) sobrevive intacta en vez de caerse sin estar listada.
- Dedup por construcción y sin lookup: misma ubicación → mismo id.
- `migrate-pending-endpoint-without-hash` se vuelve trivial: el id nunca dependió del hash, así que el reparo de `migration.md` inv. 5 se evapora.

El título del ADR aguanta: los captures siguen siendo inmutables y direccionados por contenido — por el contenido **del capture**, que es lo que `capture.md` siempre dijo que un capture era.

### `link_accepted.N` — la ubicación también se bendice

Sin este campo, `apply` repunta `link.N` y el endpoint vuelve a `OK` solo, porque el hash no cambió: **un movimiento se cierra sin que nadie lo apruebe**. Con él, `apply` deja la descripción cierta pero el endpoint no-OK hasta que un humano acepte.

> **`apply` describe, `accept` bendice** — y bendecir incluye la ubicación.

Efecto colateral útil: `link_accepted.N` mantiene vivo el capture que describe dónde estaba el fragmento **cuando se aceptó**, que es lo que hace que `git show <commit>:<file>` siga funcionando después de un rename. Hoy eso se degrada en silencio.

Costo: un rename masivo pasa a necesitar `apply -y` **y** `accept .`, dos comandos en vez de uno. Propongo que `accept` a secas bendiga **las dos dimensiones** —lo que la gente va a querer casi siempre— y que `--place` / `--content` restrinjan, para aprobar un rename sin aprobar un cambio semántico o al revés. El commit registra cuál fue.

### `link_accepted.N` según el tipo de endpoint

| Endpoint | `link_accepted.N` |
|---|---|
| estructural | id de un capture de su propia capa — resoluble localmente |
| layer · repo | **copia** del valor del vecino, opaca, no resoluble localmente |
| `task` | ausente — la ubicación es el id de la tarea y no se puede mover |
| `abstract` | ausente — no hay nada que bendecir |

Las dos primeras filas son la misma regla que hoy rige `hash.N` (*"copia del valor estructural del vecino"*), ahora con dos valores en vez de uno — así que el endpoint layer y el repo siguen siendo el mismo mecanismo, sin excepción. Pero **`capture.md` inv. 4 hay que reformularla**: habla de "referenciar captures", y ahora un campo puede contener un id de capture ajeno que nunca se resuelve localmente.

### `commit` — el commit del contenido, y es derivado

`commit` es **el commit en el que el fragmento quedó con el contenido aceptado**, no el HEAD de quien acepta (que es lo que hace hoy `accept.md`, paso 3). Con esa definición es **derivable de `(link_accepted.N, hash_accepted.N)`**, así que por los criterios de la Decisión 5 no pertenece al bilink: va a `cache/state`.

- **Cómo se calcula:** `git log -L <start>,<end>:<file>` (nativo, offline), o un walk acotado hacia atrás resolviendo la query hasta que el hash del fragmento cambie. Se calcula en `accept` —una vez, con todo el contexto a mano— y se escribe en la cache. Un derivado que escribe `accept` en vez de `check` es raro pero legítimo: sigue siendo reconstruible, que es lo único que define un derivado.
- **Arqueología:** `commit..HEAD` excluye `commit`, así que la ventana queda exactamente en "lo que pasó después de que el contenido aceptado quedó establecido". Correcto sin ajustes.
- **Fast-path:** `git diff --name-only <commit> -- <file>` sigue valiendo, con cache tibia; con cache fría se corre el algoritmo completo, que es correcto y más lento.
- **D3 se ablanda:** que el commit no sea ancestro de HEAD deja de ser verdad versionada que quedó mal y pasa a ser cache fría que se recalcula.
- **Consecuencia dura:** ese commit **no existe si el fragmento no está commiteado**. `accept` tiene que fallar sobre un archivo sucio (ver D4). Deja de ser recomendación y pasa a ser requisito.

### La frontera hereda la misma forma

El consumidor copia los dos valores del bilink remoto:

```
# retinar/.bilink/<uuid>.bilink
link.0:              @hsi
link.1:              capture <id-local>
link_accepted.0:     <id del capture del proveedor>      ← ubicación publicada
hash_accepted.0:     <hash del fragmento del proveedor>  ← contenido publicado
link_accepted.1:     capture <id-local>
hash_accepted.1:     <sha256>
```

Dos SHA-256 opacos: ninguno revela path, query, texto ni commit del proveedor. Se comparan por separado, así que la frontera obtiene el mismo vocabulario que el caso local **sin abrir el clon**:

| Comparación | Significado |
|---|---|
| las dos iguales | `OK` |
| `link_accepted.0` cambió, `hash_accepted.0` igual | el proveedor **movió** el fragmento y lo aceptó — revisar |
| `hash_accepted.0` cambió | el proveedor **cambió** el fragmento — revisar |

Un `apply` del proveedor **sin aceptar** no mueve nada: es trabajo en curso, y el consumidor no tiene por qué enterarse. Ésa es la razón de copiar campos y no el hash del archivo `.bilink` remoto entero — que además fuerza definir una forma canónica (el formato admite comentarios y no fija orden de claves) y colapsa `REJECTED` en el mismo token.

El consumidor lee además `link.1` del remoto para verificar que **sigue siendo `abstract`**; si dejó de serlo, es `REJECTED`. Dos lecturas, dos hechos, cero normalización.

Ningún commit del proveedor se copia: el escenario `consumer-stores-nothing-about-provider` exige `contains_provider_commit: false`. Se descubre recorriendo su ref hacia atrás hasta que sus `_accepted` coincidan con los aceptados — lo que la Decisión 4 § "Profundidad a pedido" ya describe.

---

## 2. La ref (H1 · H2 · D1 · D3 · D4 · D5 · D6)

### Invariante de fidelidad del snapshot

Reemplaza al argumento del `ALTERED` falso, que no se sostenía. Son dos enunciados, los dos verificables sin tree-sitter:

> **1.** El árbol de código de todo commit de `refs/bilink/<branch>` es **idéntico al de su segundo padre**.
> **2.** `accept` y `apply` absorben el commit del proyecto contra el cual calcularon su trabajo, antes de commitear.

El primero es una comparación de tree oids: exacta, O(1), y es lo que garantiza que el consumidor remoto vea exactamente lo que el proveedor tenía en ese commit — ni una foto vieja ni una incoherente. El segundo garantiza que ese segundo padre sea **el commit correcto**.

Con eso, **absorber deja de ser un comportamiento por comando y pasa a ser precondición de todo commit sobre la ref**. La tabla de tres filas de la Decisión 6 se colapsa en una regla, y se disuelve H5 (ya no hay que explicar qué commit absorbe `apply`).

**Lo que la invariante NO dice**, y no debe decir: que los bilinks del commit estén en `OK`. Una versión más fuerte —*"todo `link_accepted.N` resuelve, contra el árbol de ese commit, a su `hash_accepted.N`"*— prohibiría que la ref contenga drift, cuando el drift es el estado normal que la herramienta existe para reportar. Con `bilinker track` queda evidente: una rama nueva hereda bilinks que describen código de otro commit, y casi seguro arranca con drift. Correcto que lo haga.

### Cómo se arma el commit de la ref

```
read-tree   <commit del proyecto que se absorbe>
update-index  únicamente .bilink/
```

Nada del árbol de trabajo fuera de `.bilink/` entra jamás al commit de la ref. `cache/`, `index/` y `.bilink-migrate-*` quedan fuera del índice de bilinker, no sólo del índice del proyecto.

### Verificaciones antes de cada commit

1. **Disyunción.** El diff del commit del proyecto que se absorbe no toca `.bilink/`. Si lo toca, alguien mergeó la ref al proyecto o commiteó bilinks a mano.
   Reparación: `bilinker repair --unmerge` — commit en la rama del proyecto que vuelve a borrar `.bilink/`, y absorber ése.
2. **Fidelidad.** El árbol de código del commit nuevo es idéntico al de su segundo padre. Comparación de tree oids; si falla, abortar.

Más `bilinker verify-ref [<ref>]` explícito, para correrlo sobre una ref traída de un proveedor — que es donde más importa, porque ahí no hay otra fuente contra la cual contrastar.

### Autoría, atestación y autorización

Tres cosas distintas que conviene no mezclar:

| | Qué es | Con qué se hace | ¿Existe hoy? |
|---|---|---|---|
| **Atribución** | quién *dice* que fue | `author`/`committer` del commit de la ref | no — `accept` no registra a nadie |
| **Atestación** | prueba de que fue | `git commit -S` · `gpg.format=ssh` · `git verify-commit` | disponible, offline, sin infraestructura |
| **Autorización** | si tenía derecho a aceptar | gobernanza | no, y está bien que todavía no |

Para auditar y revertir alcanza con la atribución; ante alguien que no confía, con la firma. `git log --first-parent refs/bilink/<branch>` responde "quién aceptó qué y cuándo", y deshacer una aceptación es un `git revert` sobre esa ref. Vale una línea aclarando que el autor de git es auto-declarado: lo que constituye atestación es la firma, no el campo.

**Granularidad: un commit por invocación de `accept`, no por aceptación.** La granularidad sigue al **acto**, no al objeto. `accept <uuid>.0` da un commit; `accept .` da un commit, con el mensaje enumerando los endpoints — porque es una persona mirando y decidiendo **una vez**, y partirlo en N commits firmados no agrega verdad, sugiere N decisiones donde hubo una.

Encaja con lo que la Decisión 7 ya exige: después de reescribir las specs los NO-OK no se aceptan a ciegas, se cierran endpoint por endpoint. En ese flujo los commits salen individuales solos. `accept .` queda para lo legítimamente masivo — endpoints `PENDING` de una cadena nueva, propagación `CHAIN_DIRTY` — donde el lote sí es la decisión.

Deshacer una aceptación no necesita `git revert`: es **reescribir sus `_accepted` con los valores anteriores**, un commit nuevo, y funciona igual venga de un lote o de un commit individual. Los valores viejos se leen de `refs/bilink/<branch>~n`. Absorber sigue siendo una sola vez antes del lote.

`--first-parent` sobre la ref pasa a ser el registro de decisiones del proyecto.

**No hay riesgo de colisión de hashes por volumen.** SHA-1 son 160 bits y el límite de cumpleaños está en ~2⁸⁰ objetos; el kernel de Linux tiene ~2²³. Desde git 2.13 el algoritmo es SHA-1DC, que detecta las construcciones tipo SHAttered y aborta. Y los hashes propios de bilinker —ids de capture, `hash`, `hash_ast`— ya son SHA-256, así que el direccionamiento por contenido del ADR es más fuerte que la capa de objetos de git. Lo único que crece de forma relevante es la longitud del walk hacia atrás de `get --diff`, y como la ref es lineal y append-only eso se resuelve con bisección si alguna vez llega a importar.

**Bendecir lo mismo no diluye la responsabilidad.** Dos bilinks pueden tener `_accepted` idénticos, pero los actos que los escribieron son dos commits distintos, cada uno con su autor y su firma. La responsabilidad vive en el commit que escribió el valor, no en el valor.

**La superficie de revisión es la de bilinker, no la de la forja.** "Lo que cuesta la Decisión 6" hoy sólo nombra `sync`; hay que sumar `status`, `diff` y `log` sobre el índice/ref propios.

### Orden del corte, sin conflictos

```
1. Generar (o regenerar) .bilink-migrate-003…/ en todas las capas.      [sin git]
2. Rama del proyecto: UN commit que borra .bilink/ de todas las capas.   → X
   (pushear antes de seguir, o dos personas crean refs divergentes)
3. git update-ref refs/bilink/<branch> X                                  ← la ref nace en X
4. Renombrar .bilink-migrate-003…/ → .bilink/ en el árbol de trabajo.
5. .git/info/exclude ← .bilink/   (.bilink-migrate-* ya está desde el paso 1)
6. Commit sobre refs/bilink/<branch> con el índice propio: agrega .bilink/.  → ●, padre X
7. Escribir .accreta/migrations.
```

**Regla general:** `refs/bilink/<branch>` se crea desde un commit del proyecto en el que `.bilink/` **ya no existe**. Así la base de merge de todo `sync` futuro es `X` o un descendiente, el proyecto no vuelve a tocar `.bilink/` nunca, y el argumento de conjuntos disjuntos se cumple desde `X`. Sin ese orden, el primer merge después del corte produce un modify/delete masivo (o peor: borra en silencio los bilinks que no cambiaron).

Matiz de redacción: *"Revertir es no cortar"* vale hasta el paso 1. Del 2 en adelante revertir es `git revert X` + borrar la ref — barato igual, pero ya no es no-hacer-nada.

### Auditoría: contra la ref, no contra la rama

La rama del proyecto no está protegida: se rebasea, se force-pushea, se cambia. Pero la ref **absorbe los commits del proyecto como segundos padres**, así que todo commit alguna vez absorbido queda alcanzable desde la ref para siempre. La arqueología corre ahí:

```
git merge-base --is-ancestor <commit> refs/bilink/<branch>   ← precondición
git log <commit>..refs/bilink/<branch> -- <file>              ← la ventana
```

Si aun así no es ancestro (commit nunca absorbido, clon superficial del proveedor), degradar a *"fuente desconocida"* en vez de devolver un rango inflado.

### `bilinker track` — pendiente de decisión

Sin él, empezar a seguir una rama nueva deja todos los endpoints en `PENDING`. Compone directo con `architecture.md` § "Implementaciones alternativas por branch", que es la reconciliación que el ADR ya se debe.

```
bilinker track <branch> [--from <bilink-ref>]

1. Con --from explícito: se usa y listo.
2. Sin él, buscar entre refs/bilink/* el commit M tal que:
     · M está en la cadena de primeros padres de alguna refs/bilink/<X>
     · su segundo padre P cumple  git merge-base --is-ancestor P <branch>
     · P es el más nuevo de los que cumplen
3. Crear refs/bilink/<branch>:  primer padre M   (hereda los bilinks)
                                segundo padre    tip de <branch>  (absorbe el código)
```

**La búsqueda va de la ref hacia el proyecto, nunca al revés.** Ningún commit del proyecto tiene un merge a `refs/bilink/*` — la relación es exactamente la inversa, y esa es la invariante de D6. Buscar en los ancestros de la rama nueva un merge hacia la ref sólo puede encontrar el bug que `repair --unmerge` arregla.

El test `--is-ancestor` es lo que traduce *"la última versión de los bilinks accesible"* a algo exacto. En el caso común —la rama sale del tip de una rama trackeada y la ref está al día— el `P` más nuevo es el commit base de la rama nueva y el paso 2 termina en la primera iteración. Los tres casos donde el filtro deja de ser cosmético:

| Situación | Qué pasa sin el test | Qué hace `track` |
|---|---|---|
| La ref va **adelante** del punto de fork (rama sale de `D`, la ref absorbió `E`) | hereda bilinks que apuntan a código que la rama no tiene → `UNRESOLVED` y `ALTERED` falsos desde el minuto cero | toma el `M` cuyo `P` es `D` |
| **Ningún `P` califica** (la rama sale de antes del corte, o de una línea nunca trackeada) | elegiría algo parecido | lo dice y crea la ref desde cero |
| **Califican varias refs** (el fork es ancestro de `main` y de `rc-2.35`) | adivina | exige `--from` |

---

## 3. Estados (D2 · el estado nuevo de ubicación)

### Taxonomía de ausencia

`check` **no clona nunca** — es masivo y offline. `LAYER_UNREACHABLE` significa *"declarada y ausente; el arreglo es clonarla"*; si el clon falla, eso es un error de `stratum pull`, no un estado de bilink.

| Situación | Estado | Arreglo |
|---|---|---|
| `.toml` presente, directorio ausente | `LAYER_UNREACHABLE` | `stratum pull` |
| Ni `.toml` ni directorio, con aceptación previa | `LAYER_UNCONFIGURED` | declarar la capa · o `bilinker remove` |
| Ni `.toml` ni directorio, sin aceptación previa | `TODO` | crear la capa |
| Contenedor presente, `.bilink` del UUID ausente | `BROKEN` | investigar — es regresión |
| Repo ajeno declarado, clon ausente | `REMOTE_UNREACHABLE` | lo clona bilinker |

`UNREACHABLE` a secas **desaparece del vocabulario**. El desdoblamiento se completa sin agregar un nombre de más, y `BROKEN` conserva su significado actual de `bilink.md` sin competir con nada.

### `cache/state` — dos clases de derivado, y una invalidación que falta

La cache no está en git, así que estar fría es un estado **normal**: clon fresco, cambio de rama, después del corte, `rm -rf .bilink/cache/`, u otra máquina. Adentro conviven dos clases con garantías distintas, y el ADR las trata como una sola:

| | Qué es | Cache fría significa | Quién la escribe |
|---|---|---|---|
| `state` · `state.N` · `range` | resultado de la última verificación | **no disponible** — hay que correr `check`; es la dependencia nueva de lattice | `check` |
| `commit` | memo de un walk caro | **más lento**, nunca no disponible: se re-deriva a demanda | `accept`; re-derivable por quien lo necesite |

Por eso poner `commit` en la cache no bloquea a nadie. Donde hace falta —recuperar el texto aceptado para `EXPANDED`/`DISPLACED`/`REANCHORED`, y la atribución de "Fuente del cambio"— sólo corre sobre endpoints **ya no-OK**, así que el costo está acotado por lo que está roto y no por el total. Si la derivación falla del todo (el contenido aceptado no existe en la historia de esta rama), degrada adonde ya degrada hoy: `check.md` § "Recuperar el texto aceptado" descarta el texto y esas detecciones no corren.

**Hueco:** la Decisión 5 define `cache/state` como *uno por capa*, pero con la Decisión 6 el árbol de trabajo cambia de rama y `.bilink/` cambia con él — una cache por capa sin discriminar rama devuelve estados de otra rama en silencio. Que la cache **registre a qué commit de `refs/bilink/<branch>` corresponde** y se invalide sola si no coincide: se auto-verifica, no multiplica archivos, y no necesita saber nada de nombres de rama.

### `RELOCATED` — en qué se diferencia de `MOVED`, y por qué sale con 1

Están en **tablas distintas y en momentos distintos de la misma historia**:

| | `MOVED` | `RELOCATED` |
|---|---|---|
| Tabla | resolución — del **capture** | aceptación — del **endpoint** |
| Pregunta | ¿dónde está el fragmento? | ¿está aprobada la ubicación donde está? |
| Condición | el archivo cambió de path (git rename ≥ 50%) | `link.N ≠ link_accepted.N`, con el hash igual |
| Cuándo aparece | en el `check`, **antes** de comparar nada aceptado | **después** de que `apply` corrigió |
| Cómo se sale | `apply` | `accept` |

```
MOVED ──apply──> RELOCATED ──accept──> OK
```

`MOVED` dice *qué* pasó; `RELOCATED` dice *que nadie lo aprobó todavía*. Y `RELOCATED` es uno solo para los tres caminos que llegan ahí: `MOVED` (cambió `file`), `DISPLACED` (cambió `offset`) y `REANCHORED` (cambió `query`). El porqué vive en el capture y en el diff entre los dos captures; el estado de aceptación necesita un solo bit.

**Exit 1.** Hoy `MOVED`/`DISPLACED` salen con 0 (ADR-0002 § "Salida"), y ahí tiene sentido: son estados que `apply` cierra sin decisión humana. `RELOCATED` es lo contrario — existe precisamente porque decidimos que **los movimientos se aprueban**. Si saliera con 0, nada obliga nunca a aprobarlos: se acumulan en silencio y `link_accepted.N` queda decorativo. El código de salida es lo único que convierte "pide acción humana" en algo que un CI o un hook puede hacer cumplir.

Alternativa si alguna vez molesta que un rename ponga el CI en rojo: un **exit 2** propio, *"nada está roto, pero hay aprobación pendiente"*, que deja al consumidor elegir. Cuesta un valor más en el contrato de salida; por ahora no hace falta.

---

## 4. Correcciones puntuales

- **F2.** 158 archivos `.bilink`, 79 cadenas, **6 capas repartidas en 4 repos**:

  | Repo | Capas con bilinks | `.bilink` |
  |---|---|---|
  | raíz `accreta` | `.` · `subsystems/stratum/` · `subsystems/lattice/` | 63 + 9 + 7 |
  | `subsystems/bilinker/.stratum/impl` | una | 63 |
  | `subsystems/stratum/.stratum/impl` | una | 9 |
  | `subsystems/lattice/.stratum/impl` | una | 7 |

  Y el inventario de trabajo hay que ampliarlo: el ADR también toca `sublayer-config.md` (9 cadenas de stratum) y specs de lattice (7), más los 63 contrapartes del lado impl que van a `CHAIN_DIRTY`, no a `ALTERED`.
- **F2 bis · la migración es por repo, y son cuatro.** El ADR dice "por repo" y está bien, pero no dice cuántos: el corte de D1 se ejecuta **cuatro veces**, y quedan **cuatro `refs/bilink/main`**, una por repo. El ledger `.accreta/migrations` también es por repo (`migration.md` inv. 1), y la regla de "marcar sólo cuando corrió sobre todas las capas" sólo muerde en el repo raíz, que es el único con tres. `bilinker migrate --recursive` desde la raíz no alcanza para los otros tres: los `.stratum/impl/` están gitignoreados por sus padres y son repos independientes.
- **F4.** `check.rs:405` → `check.rs:433`. (405 es la firma de `fn check_layer`.)
- **F5.** *"`UNREACHABLE` está sólo en prosa"* → está en una tabla de `bilink.md` § "Endpoint bilink" y en prosa en `sublayer-config.md:70`. Lo cierto es que **no existe en el código**: no hay variante `Unreachable` en `EndpointState`.
- **H3.** Reemplazar *"Lo que **no** lo afecta es la Decisión 6"* por: lattice depende de bilinker y dispara lo que necesite antes de leer — la Decisión 5 agrega un `check`, la Decisión 6 agrega antes el fetch de la ref y la materialización de `.bilink/`. Dos dependencias nuevas de la misma naturaleza, no una.
- **H4.** La alternativa "el capture guarda el texto aceptado" no se rechaza por "conocimiento máximo" —el sparse-checkout ya da el archivo entero, que es más— sino porque **un chunk congelado adentro del `.capture` es una copia sin procedencia**: no se verifica, no se navega, no se refresca. Un checkout real son objetos git verificables que se actualizan en el fetch. Y agregar, marcado como no definido: traer el archivo completo es lo que a futuro dejaría a lattice recorrer las relaciones de código que salen del fragmento y evaluar impacto a profundidad N.
- **D8.** *"Sólo git y tree-sitter. `check` opera completamente offline y **nunca hace red**: si el repo ajeno no está clonado, reporta `REMOTE_UNREACHABLE` y sigue. Las operaciones de red son explícitas y puntuales: el clon inicial del proveedor, el fetch de su ref, y la profundización de `get --diff`."* Corregir también `check.md:7`.
- **D9.** `.git/info/exclude` recibe `.bilink-migrate-*` **al empezar la migración**, no en el corte. Es temporal, se borra al terminar, nunca se commitea. Y la idempotencia en dos regímenes: **antes del corte** `migrate` siempre regenera (es un derivado, y regenerar es lo que recupera un `accept` hecho con el binario viejo); **después del corte** el ledger la vuelve no-op, que es lo que `migration.md` inv. 3 exige. Anotar en cada escenario en qué régimen está.
- **C2 · prune es mark & sweep, no refcount.** Raíces: todos los `link.N` y todos los `link_accepted.N` de los `.bilink` de la capa. Se barre lo no alcanzable. El índice **acelera** el marcado; no lo autoriza — es un derivado que puede estar viejo (`index.md`), y la asimetría es fatal: si sobrestima queda basura, si subestima borra un objeto vivo. Por eso tampoco sirve un contador de referencias: se desincroniza con cualquier escritura que no pase por el índice (`checkout` de otra rama, merge, `revert`, un `apply` que crashea a mitad), que es la razón por la que git usa mark & sweep y no refcounts. **Y `prune` saca del árbol, nunca de la historia:** lo barrido sigue alcanzable en `refs/bilink/<branch>~n`, que es lo que `get --diff` recorre hacia atrás y lo que el consumidor remoto necesita para ubicar el commit del proveedor. Recuperar un objeto barrido es un `git show`. Nunca corre solo. Hoy `capture prune` sólo mira `link.N`.
- **Invariante de aceptación.** Los campos `_accepted` de un endpoint están siempre presentes juntos o ausentes juntos. `PENDING` es su ausencia. Salvedades por tipo, en la tabla de §1: `link_accepted.N` está ausente para `task` y `abstract`, y `hash_ast_accepted.N` sólo existe donde hay gramática.
- **Parser.** Enunciar una sola vez: resuelve en orden **prefijos reservados** (`capture `, `task `, el del alias de repo) → **palabras reservadas** (`abstract`) → **path Stratum**. No es una propiedad de la gramática de paths, es precedencia del parser de bilinker — y por eso los tokens reservados quedan prohibidos como primer carácter de un nombre de capa.

---

## 5. Specs a tocar — lo que falta en la lista del ADR (C4)

- `commands/get.md` — `--diff` cruzando la frontera, profundización del clon.
- `commands/graph.md` — `abstract` y repo como terminadores.
- `commands/chain.md` — `chain new` con tip `abstract`.
- `commands/sync.md` — **no existe**: comando nuevo sin spec.
- `commands/track.md` — **no existe**: comando nuevo, adoptado en §7.
- `proposals/` — **directorio nuevo**, hermano de `concepts/`. Recibe lo especificado y no implementado: el endpoint de tipo bilink con sus dos casos de uso, `kind` y `name.N`. Cada uno se lleva su motivación, no sólo su definición.
- `architecture.md` — dos cosas: el defecto (a) en la línea 57 (*"Cada nodo ancla en `hash.N` el SHA-256 del `.bilink` adyacente"*) más el diagrama de propagación de las líneas 57-63 que lo repite; y la reconciliación con § "Implementaciones alternativas por branch".
- `concepts/chain.md:47` — el otro testigo del defecto (a). El archivo ya está listado; conviene nombrar la línea.
- `bilink.md` inv. 14 — cae con `resolved_at` igual que la 13; el ADR sólo nombra la 13.
- `subsystems/worklist/concepts/open-bilink.md` — contiene un `.bilink` completo en formato viejo, con `resolved_at`.
- `lattice/integration/bilinker.md` — sigue diciendo `range.N` (campo inexistente desde el split de captures); reapuntarlo al `range` del capture, ahora en `cache/state`. Su línea 55 (baseline `commit.N`) ya está bien y sirve como testigo.
- `commands/capture.md` — escribe `resolved_at` y `state: RESOLVED` en el capture; con la Decisión 5 eso se va a `cache/state`.
- `commands/accept.md` — escribe los `_accepted` (incluido el nuevo `link_accepted.N`); exige fragmento commiteado; calcula `commit` como el commit del contenido y lo escribe en `cache/state`; gana `--place` / `--content`.
- `commands/apply.md` — deja de forkear por tipo de fix; acuña captures sin restricción y repunta `link.N`; ya no devuelve el endpoint a `OK` en `MOVED`/`DISPLACED`.

---

## 6. Escenarios

**A cambiar**

- `divergence-instead-of-shared-hash` → `distinct_captures: 1`. La divergencia vive en los `_accepted`, no en captures.
- `apply-may-not-change-content` → se reformula: `apply` puede repuntar cualquier capture, pero nunca escribe un `_accepted`, así que el endpoint sigue no-OK.
- `apply-moved-repoints-every-referent` → tras `apply` el estado no vuelve a `OK`: queda en el estado de ubicación no aprobada hasta un `accept`.
- `accept-absorbs-before-committing` → cambiar la expectativa `remote_consumer_sees_false_altered` por la invariante de auto-verificación de la ref.
- `migrate-is-idempotent` y `migrate-folder-is-regenerable` → anotar el régimen (post-corte / pre-corte).
- `layer-unreachable-for-uncloned-sublayer` → sumar los casos `LAYER_UNCONFIGURED` y `TODO`.
- `cache-is-rebuildable` → precisar qué regenera qué: `state`/`state.N`/`range` los reconstruye `check`; `commit` se re-deriva a demanda y su ausencia sólo cuesta velocidad.

**A borrar**

- `migrate-fans-out-divergent-hashes` — el fan-out desaparece.

**A agregar**

- `accept-rejects-dirty-fragment` — `accept` falla si el archivo del fragmento tiene cambios sin commitear.
- `accept-is-deterministic` — dos personas aceptando el mismo contenido en HEADs distintos escriben los mismos `_accepted`.
- `move-requires-approval` — un rename cierra con `apply` **+** `accept`, nunca con `apply` solo.
- `remote-move-is-distinguishable` — el proveedor mueve el fragmento sin cambiarlo: el consumidor lo ve como movimiento, no como cambio de contenido.
- `ref-snapshot-is-faithful` — el árbol de código de cada commit de la ref es idéntico al de su segundo padre.
- `ref-may-carry-drift` — un commit de la ref puede contener bilinks no-OK sin que eso sea corrupción; `verify-ref` no se queja.
- `track-picks-newest-reachable` — con la ref adelantada respecto del punto de fork, `track` hereda el estado correspondiente al fork, no el tip.
- `track-refuses-when-ambiguous` — si el fork es ancestro de dos refs trackeadas, `track` exige `--from`.
- `cutover-first-absorb-is-clean` — el primer `sync` después del corte no conflictúa.
- `merge-back-is-detected` — `.bilink/` aparece en un commit del proyecto: se detecta y `repair --unmerge` lo revierte.
- `cache-invalidates-on-branch-change` — cambiar de rama invalida `cache/state`; `check` no devuelve estados de la rama anterior.
- `cold-cache-check-is-correct` — con `cache/state` borrada, `check` da los mismos estados que con cache tibia; sólo tarda más.

---

## 7. Decisiones tomadas

1. **Separador del endpoint repo (F3): `@`.** `link.0: @hsi`. No necesita comillas en la shell —a diferencia de `|`, `&`, `>` y `*`—, lee como *"en el repo hsi"*, y prácticamente nadie empieza un directorio con `@`. Se reusa el sigilo de las queries (`@target`), pero los contextos no se cruzan: uno es valor de endpoint, el otro es cuerpo de query. La justificación se escribe como **precedencia del parser**, no como propiedad de la gramática de paths: `paths.md` admite `@hsi` como `traditional-path` perfectamente válido.
2. **Endpoints de tipo bilink (C3): se degradan a propuesta, no se borran.** Junto con `kind` y `name.N`. Y la regla general que sale de ahí vale para todo el defecto (c):

   > **Lo que está especificado y no implementado no se borra: se mueve a `subsystems/bilinker/proposals/`.**

   La spec pasa a describir lo que existe; la idea sobrevive con su motivación intacta y vuelve cuando alguien la implemente. Eso resuelve la asimetría que quedaba —por qué `task` sobrevivía y el endpoint bilink no— sin tener que justificarla: los dos se miden con la misma vara. Va a `proposals/` el endpoint de tipo bilink con sus dos casos de uso (`kind: governs` y el bilink de tarea, que lo necesita para colgar una tarea de *la relación* y no de uno de sus extremos), más `kind` y `name.N`. `proposals/` es un directorio nuevo, hermano de `concepts/`, `commands/` y `scenarios/`.
3. **`RELOCATED`, con exit 1.** Ver §3 para la diferencia con `MOVED` y el porqué del código de salida.
4. **`bilinker track` se adopta**, con el algoritmo de §2.
5. **Los nombres no se acortan.** Queda `hash_ast_accepted.N`, no `ast_accepted.N`: la simetría con `hash_accepted.N` y el sufijo uniforme valen más que ocho caracteres.

---

## 8. Descartado, con su motivo

Para que no vuelva a proponerse sin el argumento a la vista.

- **El `hash` del fragmento en el id del capture** (la Decisión 1 tal como está escrita). Obliga a `accept` a acuñar captures, deja a `apply` sin poder arreglar `REANCHORED` ni `EXPANDED`, y fuerza el fan-out de captures de la migración `003`. Ver §1.
- **Un almacén de aceptaciones aparte.** El objeto queda vacío: su contenido sería su propio id. Ver §1 § "Por qué no hay almacén de aceptaciones".
- **`<capture>.accept`, donde la existencia del archivo significa "aceptado".** La aceptación tiene sujeto además de objeto: un bilink heredaría la aprobación de otro sin haberla mirado, y un voto valdría para todas las relaciones sobre el mismo fragmento.
- **`H(hash, hash_ast?)` como identidad de la aceptación.** Dos fragmentos distintos con texto idéntico colapsan. La aceptación es una entrada de tree `(path, blob)`, no un blob suelto.
- **El id de la aceptación como hash de un commit**, en sus tres lecturas. *El commit de la aceptación* es circular: el archivo va adentro de ese commit, así que su nombre es parte del árbol que el hash cubre. *El HEAD al aceptar* no es único (un `accept .` da N aceptaciones en el mismo HEAD) y es la semántica descartada de "el commit donde estoy parado". *El commit del contenido* tampoco es único —un commit establece el contenido de muchos fragmentos— y rompe la convergencia entre ramas.
- **Hashear el `.bilink` remoto entero** en vez de copiar campos. Dispara con el `apply` sin aceptar del proveedor —trabajo en curso que el consumidor no puede resolver—, exige forma canónica porque el formato admite comentarios y no fija orden de claves, y colapsa `REJECTED` en el mismo token.
- **Refcount en el índice para el `prune`.** Un derivado no puede autorizar un borrado. Ver §4.
- **Un commit por aceptación** en vez de por invocación de `accept`. La granularidad sigue al acto; partir un `accept .` en N commits firmados sugiere N decisiones donde hubo una. Ver §2.
