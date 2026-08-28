# ADR-0003: bilinker — Captures inmutables y bilinks abstractos

**Estado:** Propuesto **Fecha:** 2026-08-28

---

## Contexto

Un bilink hoy conecta dos fragmentos *dentro de un proyecto*, aunque crucen repos. Hace falta que conecte fragmentos de **proyectos distintos**, con una restricción fuerte: ningún extremo debe conocer al otro — ni su hash de contenido, ni su ubicación. Y del lado del proveedor van a converger muchos consumidores sobre el mismo fragmento, sin que el proveedor se entere de ninguno.

El caso que lo fuerza: `retinar` y `filasvirtuales` consumen la API pública de `hsi`. Tres repos independientes, sin submódulos, sin dependencia Maven, acoplados sólo por una URL en runtime. Hoy no hay nada que detecte drift, y ya se está pagando: `retinar` apunta `USER_PERMISSIONS` a `/public-api/user/person/from-token` deserializando `HSIRoleInfoDto[]`, pero esa ruta devuelve un objeto único — la de roles es `/public-api/user/permissions/from-token`, que `filasvirtuales` usa bien.

### Bloqueos del modelo actual

1. **El rendezvous es por UUID compartido** y la cadena se localiza por nombre de archivo. Con C consumidores y E fragmentos el proveedor acumula O(C×E) archivos, y sumar o quitar un consumidor exige un commit en su repo.
2. **Cada extremo guarda el hash de contenido del otro.** Es el mecanismo de propagación: `hash.N` de un endpoint layer es copia del `hash.N` estructural del vecino.
3. **El fan-out por capture no sirve en la frontera.** Resuelve la multiplicidad de relaciones sobre un fragmento local; cada una sigue siendo otra cadena con su propio nodo del lado del proveedor.
4. **Ningún path Stratum llega de un proyecto a otro.** `*` es "el ancestro más lejano con `.git`"; repos hermanos sin raíz común no se alcanzan. No hay identidad de capa más allá de un path del filesystem.

Los bloqueos 1, 2 y 4 son consecuencias del mismo hecho: el vínculo entre capas es un puntero mutuo por ubicación. El 3 es consecuencia de la aridad fija más la topología lineal.

### Defectos preexistentes que este ADR resuelve de paso

**(a) Contradicción sobre `hash.N` de un endpoint layer.** `bilink.md` (inv. 6), `reference.md`, `consistency.md` y `capture.md` dicen "copia del `hash.N` estructural adyacente"; `chain.md`, `check.md` (§ Endpoint layer, paso 4) y `architecture.md` dicen "SHA-256 del archivo `.bilink` adyacente". Gana `bilink.md`: lo confirman el disco —el `hash.1` del nodo spec de `b95021d2` es `838ea0a4…`, el `hash.1` estructural del nodo impl, no el SHA del archivo, `9211a4f3…`— y el código, donde `check.rs:405` usa `adj_bl.structural_hash()`. El endpoint repo sigue la misma regla (Decisión 4), así que la lectura del hash-del-archivo queda descartada en todos los casos.

**(b) `check` ensucia archivos versionados sin cambio semántico.** Al escribir este ADR, `git status` sobre `accreta` mostraba 16 `.bilink` modificados; el diff completo de cada uno era una línea de `resolved_at`.

**(c) Campos y estados especificados que la implementación descarta.** `KEYS` en `bilink.rs` no incluye `kind` ni `name.N` — se borran al reescribir, y ningún archivo real del corpus los usa. Los endpoints de tipo bilink no existen en el enum `LinkEndpoint`. Y `UNREACHABLE` está sólo en prosa, además de sobrecargado: `bilink.md` lo usa para un `.bilink` ausente y `sublayer-config.md` para una subcapa no clonada, que son escalas distintas con arreglos distintos (Decisión 4).

### Principios que la decisión respeta

Aridad fija en dos (`bilink.md` inv. 2, commit `dfd4a26`); un bilink sólo referencia captures de su propia capa (`capture.md` inv. 4); `apply` corrige ubicación y `accept` fija contenido (`capture.md`); sólo git y tree-sitter, offline; derivados regenerables y nunca fuente de verdad (`index.md` inv. 1); nada difuso cierra solo (`check.md`); y **la aceptación es un acto humano deliberado, que va a ser gobernable por consenso** (`accreta/integration/bilinker.md`).

---

## Decisión

### 1. Captures inmutables, direccionados por contenido

El nombre de un capture pasa a ser el hash de su propio contenido:

```
id = H(file, query, offset, hash, hash_ast?)
```

- `hash` — SHA-256 del texto del fragmento.
- `hash_ast` — SHA-256 de la S-expression. **Opcional**: ausente donde no hay gramática, y sin valor discriminante en prosa. Donde no está, `RESTYLED` no existe y todo cambio de texto es `ALTERED` — que en prosa es lo correcto.
- **`commit` no entra**, ni en el id ni en el capture (Decisión 2).
- **`range`, `state` y `resolved_at` salen del capture**: si estuvieran, cada `check` cambiaría el id.

**Disuelve la objeción que hoy impide bajar el hash al capture.** `capture.md` argumenta que "dos bilinks pueden haber aceptado versiones distintas del mismo fragmento […] con un solo hash compartido eso sería imposible de expresar". Deja de aplicar: si A aceptó la v1 y B la v2 **no comparten capture**. Compartir un capture pasa a significar "aceptamos lo mismo", que es más fuerte que la regla actual.

**Consecuencias directas**: `accept` se convierte en repuntar `link.N`, y el `hash.N` del bilink desaparece por redundante — está en el id. La deduplicación sale por construcción, y se va el lookup por `(file, query, offset)`. Y el **copy-on-write de `apply` desaparece como concepto**: con captures inmutables todo es fork siempre.

**No hay repunte en masa.** El capture viejo no desaparece: sigue diciendo "lo aceptado era X". Los otros bilinks que lo referencian siguen apuntando a un capture válido, y su próximo `check` resuelve `file`+`query` contra el árbol actual y da `ALTERED`. Repuntarlos automáticamente sería aceptar el cambio en nombre de todos — justo lo que el copy-on-write existe para evitar.

**La frontera entre `apply` y `accept` se vuelve aritmética:**

> `apply` sólo puede mintear un capture cuyo `hash` sea **idéntico** al del anterior. Si el hash cambia, es `accept`.

| Estado | Qué cambia | Cierra |
|---|---|---|
| `MOVED` | `file` | `apply` |
| `DISPLACED` | `offset` | `apply` |
| `REANCHORED` | `query` — hash igual, pero la inferencia es difusa | `apply` + `accept` |
| `RESTYLED` · `EXPANDED` · `ALTERED` | el texto | `accept` |

`RESTYLED` es caro —un espacio de más mintea un capture nuevo— y está bien que lo sea: que el AST no cambie describe *qué* cambió, no que el cambio esté aprobado. Con la aceptación votable a futuro, cualquier cosa que `apply` cierre solo es una decisión que se saltea el consenso.

**La identidad a través del tiempo se reubica.** Un id de capture deja de significar "el método `vote`" y pasa a ser una instantánea. Lo durable es el **UUID del bilink**. Es el modelo de git: el path es la identidad, el hash es el snapshot.

### 2. `commit` va con la aceptación

No es propiedad del fragmento sino procedencia de la decisión. Si entrara al id, el mismo fragmento aceptado en dos commits distintos daría dos captures idénticos con ids distintos, rompiendo la deduplicación y el significado de compartir.

Consecuencia: el texto aceptado se recupera con `git show <commit>:<file>` dentro de un repo. Cruzando la frontera **no se recupera por defecto**, porque el clon del proveedor es superficial: ahí los estados de `check` degradan a `OK` / `ALTERED` / `UNRESOLVED`, sin `EXPANDED`, `DISPLACED` ni `REANCHORED`. No es un límite del modelo sino del clon, y se levanta a pedido (Decisión 4, "profundidad a pedido").

### 3. El endpoint `abstract`

Una punta que no es responsabilidad de quien la declara:

```
# hsi/.bilink/<uuid>.bilink        — bilink abstracto
link.0: capture <hash>
link.1: abstract                   ← lo aporta quien lo consuma
```

Un valor, no un campo ausente: la aridad fija en dos es la invariante más deliberadamente defendida de la spec y no vale la pena romperla por eso. Hay precedente en `TODO`, que ya es "una intención declarada — no es un error"; acá la intención es permanente y abierta a cualquiera.

El proveedor necesita un bilink y no le alcanzan los captures: un capture tiene estado de resolución pero no de aceptación, y *"lo que publiqué sigue coincidiendo con lo que aprobé"* es una pregunta real y puramente local que sólo un bilink sostiene. Además le da una identidad durable que sobrevive a sus propios cambios, y que es lo que los consumidores referencian.

`abstract` es palabra reservada y se chequea **antes** del fallback *"ninguna de las anteriores → Layer"*, o se parsearía como path Stratum.

**`state.N` de una punta `abstract` es `OPEN`**: constante, siempre sana, nunca pide acción. No hay contra qué compararla, así que no puede tomar otro valor. Se le da un nombre en vez de dejar el slot vacío porque la tupla `(state.0, state.1)` la consumen `check` para su código de salida, `accept .` para elegir a quién aceptar, `status` para imprimir y lattice para el campo `state` de la arista: un valor constante lo maneja cada uno sin ramas, un hueco obliga a todos a tratar el caso nulo. Es la misma forma que `TODO`. `accept .` nunca la toca.

### 4. El endpoint repo

El UUID es el mismo que el del bilink remoto, así que no se escribe; y el otro repo se nombra por un alias local:

```
# retinar/.bilink/<uuid>.bilink
link.0: |hsi
link.1: capture <hash-local>
```

La barra inicial marca el cruce de frontera. Stratum usa `>` (abajo), `<` (arriba) y `*` (raíz) para navegar **dentro** del árbol; `|` no navega, salta afuera, y ningún path Stratum válido empieza así. Un alias desnudo caería en el fallback *"ninguna de las anteriores → Layer"* y se resolvería como directorio relativo; con la barra la discriminación es un carácter. Como `>` y `*`, hay que comillarlo en la shell.

El alias se declara en un `.toml` con la misma forma que hoy tienen las carpetas `.stratum` (`.stratum/.impl.toml`), aplicada a un repo que no es una subcapa:

```toml
# .bilink/.hsi.toml
remote = "git@gitlab…:minsal/hsi.git"
branch = "rc-2.32"
```

**Vive en `.bilink/`, no en `.stratum/`**: un proveedor externo no es una capa inferior del consumidor, y declararlo bajo `.stratum/` diría que sí. El clon va al lado de su declaración, en `.bilink/<alias>/`, y está gitignoreado — no se commitea el checkout de otro repo.

**El endpoint repo es el endpoint layer generalizado.** Misma convención de UUID compartido, mismo `.bilink/` implícito, misma forma de resolución — sólo cambia que la dirección se resuelve por alias en vez de por path relativo:

```
layer:  resolved = ../<layer-path>/.bilink/<uuid>.bilink
repo:   resolved = <clon de .{alias}.toml @ refs/bilink/{branch}>/.bilink/<uuid>.bilink
```

El `.toml` declara la rama **del proyecto**, y la herramienta traduce a su ref de bilinks (Decisión 6): `branch = "rc-2.32"` se busca en `refs/bilink/rc-2.32`. Una sola fuente de verdad, y la traducción es trabajo de bilinker. Como esa ref lleva el árbol del proyecto más `.bilink/`, un solo fetch trae las declaraciones del proveedor y el código al que apuntan, coherentes entre sí.

**Lo que gana la indirección por alias**: el `.bilink` no contiene ninguna URL, sólo un nombre local. Toda la identidad del proveedor —dónde está, qué rama— queda concentrada en un archivo por proveedor, que es el único lugar del consumidor que sabe algo del otro repo. Si `hsi` cambia de host, se edita un archivo y no N bilinks.

**Lo aceptado es el `link.0` del bilink remoto, no el hash de su archivo** — es decir, el id del capture que el proveedor está publicando. Hashear el archivo entero haría que editar un `name.0` ensuciara a todos los consumidores, y obligaría a definir una forma canónica para evitarlo; copiar un solo campo no necesita nada de eso.

Y así el endpoint repo sigue **la misma regla que el endpoint layer** — copiar el valor estructural del vecino — sin ninguna excepción. Bajo la Decisión 1 el valor estructural aceptado *es* el id del capture, así que "copiar el `hash.N` estructural adyacente" y "copiar el `link.0` adyacente" son la misma operación.

Ese token:

- es **opaco** — es un hash, y no revela path, query, texto ni commit del proveedor;
- cambia **exactamente** cuando cambia el fragmento publicado, y por ninguna otra razón: es inmune a etiquetas, comentarios y reordenamientos del archivo remoto;
- **no invalida la referencia** cuando el proveedor evoluciona, porque `link.N` sigue apuntando al UUID durable y sólo se mueve el valor aceptado. Poner el id del capture en el *endpoint* dejaría la referencia colgada en cada cambio, sin distinguir "cambió" de "nunca existió"; ponerlo en la *aceptación* no;
- da **fan-out gratis**: el proveedor tiene **un** archivo, y cada consumidor tiene el suyo con el mismo nombre en su propio repo. Ninguno sabe del otro; el proveedor no sabe de nadie.

**El pin de versión es la branch, declarada una sola vez.** `branch` vive en el `.toml` y no en cada bilink: todos los vínculos de `retinar` hacia `hsi` siguen la misma rama, y cambiar de `main` a `rc-2.32` es editar una línea. Combinado con `architecture.md` § "Implementaciones alternativas por branch", soportar dos versiones del proveedor a la vez son dos branches del consumidor, cada una con su `.hsi.toml` — no un campo de rango de versiones. La branch dice **qué línea seguir**; el token aceptado dice **qué vi en esa línea**.

**Clona bilinker, y el sparse lo calcula, no lo declara.** `sublayer-config.md` define `sparse` como un campo que el humano escribe, y acá eso sería un valor derivado metido en un archivo de declaración — el mismo error que la Decisión 5 corrige en todos los demás archivos. El conjunto de archivos a traer sale de los bilinks:

1. Clonar sólo `.bilink/` — alcanza para el paso siguiente.
2. Por cada bilink local con endpoint repo hacia el alias, resolver `<clon>/.bilink/<uuid>.bilink`, seguir su endpoint estructural hasta `<clon>/.bilink/capture/<hash>.capture`, y leer su `file`.
3. Ampliar el sparse-checkout a ese conjunto (`git sparse-checkout set`), sin volver a clonar.

No hace falta persistir el conjunto: git ya lo guarda en el clon, que además está gitignoreado. Y es **incremental por naturaleza** — sumar un vínculo de frontera agrega un archivo, sacarlo lo quita. Un conjunto fijo en el `.toml` quedaría desactualizado con el primer bilink nuevo.

Hacen falta los archivos y no sólo los `.bilink` porque detectar el drift y **entenderlo** son cosas distintas: el token dice que algo cambió, pero para mirar el fragmento, correr `get` y decidir si se acepta hay que tener el archivo del proveedor.

**Profundidad a pedido.** El clon arranca superficial: el árbol actual de la rama declarada, sin historia. Alcanza para `check`, que corre sobre todo y no puede andar profundizando clones como efecto colateral. Cuando alguien pide ver qué cambió entre lo aceptado y lo actual de **un** bilink, recién ahí se trae lo necesario: se recorre el `.bilink` remoto hacia atrás hasta la versión cuyo `link.0` coincide con el token aceptado, se lee el capture de entonces, y se compara. Git lo soporta directo —`fetch --deepen`, o un clon parcial con `--filter=blob:none` que trae los blobs sólo cuando se los toca— y el costo se paga únicamente donde hay un humano mirando. Más adelante puede haber una flag tipo `--all` que traiga todo de entrada.

Ése es el reparto correcto: **`check` es masivo y barato; ver el diff es puntual y caro.** El conocimiento mínimo queda como default, no como límite.

**Del otro lado no hay capas.** Un alias nombra un repo, no una capa: bilinker busca el `.bilink/` de la raíz del clon y nada más. La estructura interna del proveedor —si tiene `.stratum/`, cuántas capas— no es asunto del consumidor, y meterla en el `.toml` sería volver a saber de más.

**Dos estados propios.**

`REMOTE_UNREACHABLE` cuando el proyecto del proveedor no está disponible. No alcanza con `UNREACHABLE` a secas: hoy ese nombre está sobrecargado y tapa tres situaciones que se arreglan distinto.

| Falta | Cómo se arregla | Estado |
|---|---|---|
| La subcapa, sin clonar | `stratum pull` | `LAYER_UNREACHABLE` |
| El proyecto ajeno, sin clonar | lo clona bilinker | `REMOTE_UNREACHABLE` |
| El `.bilink` referenciado, con el contenedor presente | es regresión: hay que investigar | `UNREACHABLE` |

Las dos primeras son normales —`sublayer-config.md` ya dice que trabajar sin clonar todas las capas es lo esperado— y la tercera es un problema real. Bajo un solo nombre no se puede distinguir "me falta traer algo" de "algo se rompió", que es la diferencia que decide si alguien tiene que mirar. Y no basta con separar en dos: **la subcapa y el proyecto ajeno también se arreglan con comandos distintos**, así que comparten poco más que la causa.

El nombre sale de qué es la cosa, no de cómo se la trae. Una capa es una capa aunque su `.toml` la busque por URL; un proyecto git ajeno es, en el vocabulario de git, un remoto. No sirve `REPO_`, porque una capa Stratum **también** es un repo —`layer-model.md`: "cada proyecto y cada capa interna es un repositorio git independiente"— así que no contrastaría con nada. Tampoco el `external` de lattice, que nombra un URI http inverificable con garantía `asserted`: un repo ajeno se clona, se hashea y se verifica igual que lo propio.

`LAYER_UNREACHABLE` es parte de esta decisión aunque no sea parte de la frontera: aplica al endpoint layer y aparece sin que haya ningún proyecto ajeno de por medio. Dejarlo afuera dejaría el desdoblamiento a medias —dos de los tres casos separados y el tercero todavía sobrecargado— y obligaría a volver sobre las mismas tablas de estado dos veces. `sublayer-config.md`, que hoy dice que una subcapa no clonada reporta `UNREACHABLE`, se corrige acá.

`REJECTED` cuando el `link.1` remoto deja de ser `abstract`. La otra punta ya no admite ser ampliada, así que el vínculo no puede sostenerse — es un hecho distinto de "el fragmento cambió" y no debe mezclarse en el mismo token. El nombre describe la condición desde el lado que la sufre, que es el único que la puede observar: el proveedor no rechaza a nadie en particular, ni sabe que hay alguien.

**Conocimiento unidireccional.** El consumidor nombra al proveedor, y eso es inevitable y correcto: hay que saber de qué se depende. Lo que importa es que el proveedor no sabe de nadie.

**Esto enmienda un principio.** `configuration.md` dice hoy "No existe `.bilinker.toml` ni ningún otro archivo de configuración". Deja de ser cierto, y conviene precisar qué cambia y qué no: la raíz se sigue descubriendo caminando hacia arriba, y el lenguaje se sigue infiriendo de la extensión — no aparece configuración *de la herramienta*. Lo que aparece es una declaración *del proyecto* sobre de qué depende, que es contenido, igual que un `.stratum/.impl.toml`. Pero la frase absoluta hay que reescribirla.

### 5. Partición de archivos

Un archivo por *quién lo escribe y qué significa*. Las decisiones 1 y 4 lo vuelven obligatorio, no opcional.

| Archivo | Contenido | Escribe | git |
|---|---|---|---|
| `<uuid>.bilink` | `link.0`, `link.1` — declaración | `chain new` · repunte de `apply`/`accept` | sí |
| `<hash>.capture` | `file`, `query`, `offset`, `hash`, `hash_ast` — inmutable | `capture` · `apply` | sí |
| `<uuid>.accept` | `commit.N` · `accepted.N` para endpoints no estructurales — la decisión | `accept` | sí |
| `cache/state` | `range`, `state`, `state.N`, `resolved_at` — derivado, uno por capa | `check` | no |

Ningún archivo de bilinker se escribe a mano: todos salen de un comando.

**`kind` y `name.N` salen del formato.** Están especificados pero no implementados —`KEYS` los descarta al reescribir— y ningún archivo real los usa. Este ADR no los necesita: al ser el valor aceptado el `link.0` remoto y no el hash del archivo, ya no influyen en nada. Y para un bilink abstracto son directamente vacíos: `name.1` nombraría una punta que no existe, y `kind` clasificaría una relación declarada a medias. Se quitan hasta que algo los necesite; `kind: governs`, que es su único caso de uso documentado, puede volver con su propia decisión y su propia implementación.

**El sufijo `.N` sobrevive sólo donde el dato es de una punta.** `hash` y `hash_ast` lo pierden porque se mudaron adentro del capture, donde hay un único fragmento y no hay qué numerar. `commit.N`, `accepted.N` y `state.N` lo conservan: cada endpoint se acepta en su propio momento y su propio repo, y `check` devuelve una tupla justamente porque un extremo puede estar `OK` y el otro `ALTERED`.

`accepted.N` sólo existe para endpoints **no estructurales**: el de un estructural ya está en el id del capture que `link.N` referencia. Para un endpoint layer es la copia del valor estructural del vecino; para uno repo, lo mismo, a través del clon.

`cache/state` como **un archivo por capa**, igual que `index/index`: reescritura atómica, menos inodos, cero conflictos de merge. `index.md` § "Git" ya sienta el precedente de un derivado dentro de `.bilink/` que puede estar gitignoreado.

`.accept` aparte no es cosmético: con la aceptación votable querés un artefacto que sea exactamente la decisión —para firmarlo, diffearlo y colgarle votos— y no un archivo donde `check` también escribe. Y en la frontera queda como el único archivo con conocimiento del otro lado.

### 6. Los bilinks viven en una ref paralela

Ninguna rama del proyecto contiene `.bilink/`. Los bilinks viven en **`refs/bilink/<branch>`**, una ref por rama del proyecto: los de `rc-2.35` están en `refs/bilink/rc-2.35`.

Lo que compra es adopción. Sin esto, la Decisión 3 le pide a un proyecto como `hsi` —con mucha gente que nunca oyó hablar de bilinker— que acepte carpetas nuevas en su rama principal. Con esto, `hsi` publica abstracciones y su `main` no cambia un byte. El costo de ser proveedor deja de ser una negociación con el equipo entero.

**Fuera de `refs/heads/`.** La ref no es una rama: `git branch -a` no la lista, la UI de la forja tampoco —los listados de ramas muestran `refs/heads/*`— y `git log --branches` la ignora. Es lo que hacen `git notes` con `refs/notes/*` y Gerrit con `refs/changes/*`. Requiere refspecs explícitos para push y fetch, que los pone bilinker; nadie los tipea. Así los bilinks no existen para quien no los busca, que es más fuerte que tenerlos en una rama aparte.

**Contenido: el árbol del proyecto más `.bilink/`.** No una rama huérfana con sólo los bilinks. Cada commit es entonces un snapshot consistente por construcción, y eso es lo que hace simple el caso remoto: el consumidor trae **una sola ref** y obtiene las declaraciones del proveedor junto con exactamente el código al que apuntan, sin tener que traer dos refs y confiar en que se correspondan. El árbol no se duplica: git comparte los objetos con la rama del proyecto, así que el costo marginal de cada snapshot es la carpeta `.bilink/` y nada más.

**Evolución: merge, y en una sola dirección.** Cuando la rama del proyecto avanza, la ref de bilinks la absorbe con un merge. Nunca rebase —reescribiría la historia que el `get --diff` de la Decisión 4 necesita recorrer hacia atrás— y nunca cherry-pick, que copia los commits en vez de referenciarlos: los del proyecto dejarían de ser ancestros, cada ronda siguiente conflictuaría por falta de base común, y se perdería la correspondencia. **El merge nunca se hace al revés.** Si la ref de bilinks se mergeara de vuelta al proyecto, contaminaría justamente lo que este diseño mantiene limpio.

Los merges no conflictúan nunca: la ref de bilinks **no modifica archivos del proyecto** —su único diff es agregar `.bilink/`— y el proyecto no toca `.bilink/`. Los dos lados escriben conjuntos disjuntos.

```
rc-2.35:              A ── B ── C ─────── D ──── E
                       \          \        \      \
refs/bilink/rc-2.35:    ●───────── M1 ───── M2 ─── M3
                        +.bilink   ↑        ↑      ↑
                                   merge C  merge D  merge E
```

**La correspondencia con el proyecto es el segundo padre**, y por lo tanto un hecho de git y no una convención de nombres: se recorre con `git log --parents`, y `git branch --contains` y `git merge-base` la responden solas. No hace falta ningún identificador propio — el hash del commit ya lo es, y el estado anterior es `refs/bilink/rc-2.35~1`.

Y se lee bien: **`git log --first-parent refs/bilink/rc-2.35` muestra sólo la evolución de los bilinks**, ocultando la historia del proyecto absorbida.

**Los `.bilink/` están en el árbol de trabajo, no en un worktree aparte.** Quien usa bilinker o lattice quiere esos archivos a mano, así que se materializan junto al código, y el proyecto los ignora. La exclusión va en **`.git/info/exclude`, no en `.gitignore`**: `.gitignore` está versionado, y agregarlo modificaría la rama del proyecto — justo lo que este diseño evita. `info/exclude` es local, no se commitea y no aparece en ningún MR.

Con eso `check` corre en caliente: bilinks y código vivo en el mismo directorio, con los cambios sin commitear a la vista. Es lo que `check.md` ya exige al comparar contra el árbol de trabajo y no contra HEAD, *"porque los cambios sin commitear quedan invisibles — que es el caso más común mientras alguien trabaja"*. Si se comparara contra la copia de código de la ref, quien rompe un vínculo no se enteraría hasta que alguien sincronice, y la detección de drift llegaría siempre tarde.

**Un índice propio.** Ignorados a secas, los cambios que escribe `accept` no aparecerían en ningún `git status`: para el índice del proyecto son archivos ignorados, y la ref donde cuentan no está checkouteada. Bilinker usa su propio `GIT_INDEX_FILE` sobre el mismo árbol de trabajo, contra `refs/bilink/<branch>`. El mismo `.bilink/` queda ignorado por el índice del proyecto y trackeado por el de bilinker, que así recupera `status` y `diff` reales sin ensuciar los del proyecto. Es el patrón conocido de los dotfiles en repo bare.

**Quién toca la ref, y cuándo absorbe.**

| Operación | Commitea en la ref | Absorbe el proyecto |
|---|---|---|
| `check` | no — su salida va a `cache/state`, fuera de git | no |
| `accept` · `apply` | sí | sí, el commit contra el que se acepta |
| `sync` | sí | sí |

`check` no produce ningún commit: por la Decisión 5 el estado es derivado y vive fuera de git. Atarlo a un commit reintroduciría el defecto (b) empeorado — un commit por corrida en vez de 16 archivos sucios.

**`accept` y `apply` tienen que absorber, no alcanza con `sync`.** Si commitearan sin hacerlo, la ref quedaría con bilinks nuevos sobre código viejo. Con el proyecto en `E` y la ref en `C`: `commit.N` registra `E` y el hash aceptado es el del contenido en `E`, pero el árbol de la ref tiene `C`. Un consumidor remoto —que sólo tiene la ref— resuelve el capture contra `C`, hashea, no coincide, y **reporta un `ALTERED` falso**. No es una foto vieja: es una foto incoherente. Y el commit que hay que absorber es justamente el que `commit.N` ya registra.

`sync` cubre el otro caso: el proyecto avanzó y nadie aceptó nada. Alinea la ref con el proyecto y **no verifica nada** — de ahí el nombre; `update` sugeriría que recalcula estados, que es lo que no hace.

**La asimetría local/remoto, dicha explícitamente.** Localmente el código sale del árbol de trabajo, así que la foto de la ref puede estar atrasada sin afectar un `check`. Remotamente el consumidor sólo tiene la ref, así que esa foto **es** el código con el que verifica. Por eso el merge no es un requisito para observar, sino para que el snapshot sea cierto para quien no tiene otra cosa.

### 7. Migración

`concepts/migration.md` ya define la maquinaria: ids ordenados, ledger por repo en `.accreta/migrations`, runner idempotente, `--dry-run` que no escribe, y la regla de que una migración se marca aplicada sólo cuando corrió sobre **todas** las capas del repo. Las dos nuevas se registran en `commands/migrate.md` junto a `bilinker-001-capture-split`.

**El orden importa y no es el obvio.** La partición va primero: mientras `range`, `state` y `resolved_at` sigan dentro del `.capture`, no se le puede calcular un id estable.

**`bilinker-002-file-partition`** — parte cada `.bilink` en tres archivos. `hash.N`, `hash_ast.N` y `commit.N` van a `<uuid>.accept`; `state.N` y `resolved_at` van a `cache/state`; `range`, `state` y `resolved_at` salen del `.capture` hacia el mismo `cache/state`. El `.bilink` queda con `link.0`, `link.1` y los campos semánticos.

**`bilinker-003-immutable-captures`** — renombra cada `.capture` a `H(file, query, offset, hash, hash_ast?)` y repunta los `link.N`. Tiene dos casos que no son un renombre:

- **Fan-out.** Un capture referenciado por varios bilinks con `hash.N` distintos se parte en **un capture por hash aceptado distinto**, porque bajo el modelo nuevo aceptar cosas distintas es tener captures distintos. Es la inversa exacta de `bilinker-001`, que deduplicaba: acá la duplicación es la respuesta correcta y hay que reintroducirla.
- **Endpoints sin aceptar.** Un endpoint en `PENDING` no tiene `hash.N`, y la migración **no puede calcularlo**: `migration.md` inv. 5 prohíbe resolver queries y consultar git. El id se computa sobre los campos presentes (`file`, `query`, `offset`), queda bien definido, y el primer `accept` mintea el definitivo.

**No hace falta migración para las decisiones 3 y 4.** Los endpoints `abstract` y repo son aditivos: ningún archivo existente los usa, y todos siguen siendo válidos. La frontera se puede adoptar bilink por bilink, sin tocar nada de lo que ya está.

**La Decisión 6 tampoco es una migración, y no puede serlo.** Mover los bilinks a `refs/bilink/<branch>` no transforma ningún archivo: los deja idénticos y cambia dónde viven. Y `migration.md` inv. 5 prohíbe que una migración consulte git, que es todo lo que esta operación hace. Es un paso único y aparte, por repo.

**Va último.** Si la mudanza corriera primero, las dos migraciones tendrían que operar sobre archivos que no están en el árbol de trabajo, con worktree o plumbing. Corriendo al revés —`002`, `003`, y recién después la mudanza— las migraciones siguen siendo lo que son hoy: transformaciones de archivos comunes, en el árbol, verificables con `git diff` antes de commitear.

El corolario práctico de `migration.md` aplica igual: `bilinker migrate --recursive` desde la raíz, nunca invocaciones sueltas por capa, o el repo queda marcado con capas sin migrar.

---

## Consecuencias

**Invariantes nuevas.** El nombre de un capture es el hash de su contenido, y un capture es inmutable · `apply` sólo mintea captures de `hash` idéntico · un endpoint `abstract` no tiene ni va a tener contraparte en su propio repo · un endpoint repo se nombra por alias con `|`, y el alias se resuelve por un `.bilink/.{alias}.toml` · los bilinks viven en `refs/bilink/<branch>` y ninguna rama del proyecto los contiene · esa ref nunca modifica archivos del proyecto, y nunca se mergea de vuelta.

**Invariantes enmendadas.** La aridad sigue en dos: `abstract` es un valor, no una ausencia · `hash.N`/`commit.N` juntos sólo para estructural, layer y task · queda la versión de `bilink.md` para el hash del endpoint layer · una cadena tiene exactamente dos terminadores, cada uno un tip estructural, un `abstract` o un endpoint repo, y sigue siendo lineal: la bifurcación ocurre *entre* cadenas, en el UUID del bilink abstracto · **cae "no existe ningún archivo de configuración"**, aunque la raíz y el lenguaje se sigan resolviendo sin ella.

**Se va.** El copy-on-write de `apply` y la regla de fork por tipo de fix.

**Lo que cuesta la Decisión 6.** Bilinker pasa a manejar su propio índice git y sus propios refspecs, y gana un comando: `sync`. Las escrituras siguen siendo I/O de archivos normal sobre el árbol de trabajo —los `.bilink/` están ahí, sólo que excluidos del índice del proyecto— así que no hace falta plumbing ni un worktree aparte. Lo que sí cambia de naturaleza es que la herramienta ahora administra una ref: crearla, absorber, empujar y traer. El ledger `.accreta/migrations` la acompaña, porque es metadata de bilinker sobre archivos de bilinker.

**Hay que reconciliar con "Implementaciones alternativas por branch".** `architecture.md` ya usa el pareo de ramas para otro eje: `specs/feature/X` ↔ `impl/feature/X` es variación entre alternativas, y la Decisión 6 es separación entre bilinks y contenido. Componen —`refs/bilink/feature/X`— pero el documento tiene que decirlo, o los dos usos del mismo mecanismo se leen como uno solo.

**Efecto por comando.** `capture` devuelve un hash · `check` compara contra el `hash` del capture y no contra `hash.N`, con rama nueva para el endpoint repo, escribe a `cache/state` y no commitea nada · `accept` mintea, repunta, escribe `commit.N`/`accepted.N` en `.accept` y absorbe el commit que acepta · `apply` igual, con la restricción de hash idéntico · `get --diff` cruzando la frontera profundiza el clon a pedido · `graph` emite `abstract` y repo como terminadores · `index` sin cambios · `chain new` acepta un tip `abstract` · **`sync` es nuevo**: alinea la ref con el proyecto sin verificar nada.

**Estados nuevos.** `OPEN` (punta `abstract`, constante y sana), `REJECTED` (la otra punta dejó de ser abstracta), `REMOTE_UNREACHABLE` (el proyecto del proveedor no está clonado) y `LAYER_UNREACHABLE` (la subcapa no está clonada). Este último no es de la frontera: desdobla un `UNREACHABLE` que ya estaba sobrecargado, y lo deja significando una sola cosa — el `.bilink` referenciado no está, con el contenedor presente. Tres nombres para tres arreglos distintos: `stratum pull`, clonar el remoto, e investigar.

**Limitación conocida.** Con el UUID implícito, un consumidor no puede vincular **dos** fragmentos locales a la misma abstracción remota: el nombre de archivo colisiona. Haría falta una segunda abstracción del lado del proveedor, y ahí el bloqueo 1 reaparece parcialmente. Se acepta, porque los consumidores reales ya centralizan el consumo en un único método por operación.

**Trabajo que dispara en las specs.** Sanear los defectos (a), (b) y (c) · reescribir `concepts/capture.md` y `concepts/configuration.md` · crear `concepts/cache.md` y `concepts/accept.md` · ajustar `concepts/bilink.md`, `reference.md`, `chain.md`, `consistency.md` y los comandos `capture`, `check`, `accept`, `apply`, `status`, `migrate` · agregar `scenarios/frontier.yaml`. En Stratum, `sublayer-config.md`, que hoy reporta `UNREACHABLE` para una subcapa no clonada. En lattice: nodo del bilink abstracto, aristas dirigidas consumidor → abstracción, y **namespace de proyecto en el id canónico**, porque hoy `<layer-root>::<path>#<range>` es relativo a la raíz y dos proyectos con una capa en `.stratum/impl` colisionan. La implementación vive en `subsystems/bilinker/.stratum/impl/crates/bilinker/src/`.

---

## Relación con ADR-0002

ADR-0002 sigue vigente en lo estructural —referencias por nodo AST, `@target`, cadenas— pero quedó desactualizado en tres puntos que este ADR consolida: describe `.bilinker.toml` con workspaces, que `configuration.md` ya niega; describe el endpoint estructural como `workspace :: file :: query :: start~end`, previo al split de `.capture`; y enumera diez estados en el bilink, que hoy están partidos entre capture y bilink.

---

## Alternativas descartadas

- **URL de git inline en el endpoint** (`link.0: git@… rc-2.32`). Directo y sin archivo aparte, pero mete la identidad del proveedor en cada bilink y obliga a editar N archivos para cambiar de rama o de host.
- **UUID explícito en el endpoint repo.** Permitiría que el consumidor elija su propio UUID y levanta la limitación conocida, al costo de un token redundante y de romper la convención de UUID compartido que ya rige las cadenas.
- **Hashear el archivo `.bilink` remoto entero** en vez de copiar su `link.0`. Obliga a definir una forma canónica, porque si no un cambio de `name.0` ensucia a todos los consumidores.
- **Manifiesto publicado con token de revisión** (`contract publish` / `fetch`, `rev` `<mayor>.<serial>`, clasificación breaking/compatible declarada). Da semántica de compatibilidad, pero es maquinaria nueva entera para conseguir un token opaco que el `link.0` remoto ya da derivado en vez de declarado.
- **Sólo captures del lado del proveedor.** Un capture no tiene estado de aceptación, así que el proveedor no podría responder si lo publicado sigue coincidiendo con lo aprobado.
- **`link.1` ausente en vez de `abstract`.** Rompe la aridad fija en dos por un campo vacío.
- **`sparse` declarado en el `.toml`.** Es un valor derivado de los bilinks del proyecto, y quedaría viejo con el primer vínculo nuevo.
- **`hash_ast` en el id en vez de `hash`.** Un reformateo no cambiaría el id, pero tampoco se detectaría nunca, y para lenguajes sin gramática no habría id.
- **El capture guarda el texto aceptado.** Lo vuelve auto-contenido y sin arqueología de git, pero el consumidor pasaría a tener copia literal del fragmento del proveedor: conocimiento máximo, lo contrario del requisito.
- **Raíz de ecosistema**, un repo contenedor con los tres proyectos como sub-proyectos. No resuelve los bloqueos 1–3.
- **Bilinks en las ramas del proyecto**, como hoy. Obliga a cada proveedor a aceptar carpetas `.bilink/` en su rama principal, que para un proyecto con muchos participantes ajenos a bilinker es una negociación y no una decisión técnica.
- **Rebasear la ref de bilinks sobre la del proyecto.** Reescribe historia de forma permanente: los commits que el consumidor necesita alcanzar para responder "¿qué cambió desde que acepté?" dejan de existir, y cada fetch pasa a ser non-fast-forward.
- **Cherry-pick de los commits del proyecto.** Los copia en vez de referenciarlos: dejan de ser ancestros, cada ronda conflictúa por falta de base común, y se pierde la correspondencia con el proyecto.
- **Rama huérfana con sólo `.bilink/`.** Evita el merge, pero obliga al consumidor a traer dos refs —los bilinks de una y el código de otra— y a confiar en que se correspondan. El snapshot completo es coherente por construcción y no cuesta almacenamiento, porque git comparte los objetos.
- **Una rama por estado**, `bilink/<branch>/<id>` con `<id>` incremental o el hash referenciado. Reinventa el historial de commits con nombres de rama: agrega un problema de búsqueda del estado previo que `~1` ya resolvía, y llena el namespace de refs.
- **`refs/heads/bilink/*`** en vez de un namespace propio. Las ramas aparecerían en `git branch -a` y en el listado de la forja, que es justo lo que se quería evitar.
- **Excluir con `.gitignore`** en vez de `.git/info/exclude`. Está versionado, así que modificaría la rama del proyecto — una línea, pero deja de ser cierto que el proyecto no cambia.
- **Leer los bilinks desde un worktree lincado**, sin materializarlos en el árbol. Obliga a traducir paths entre dos árboles y a que `check` elija entre código vivo y código de la ref; y las herramientas que ya miran `.bilink/`, como la extensión de VS Code, no lo encontrarían donde esperan.
- **Absorber sólo en `sync`.** Deja la ref con bilinks nuevos sobre código viejo entre sincronizaciones, y un consumidor remoto reporta `ALTERED` falsos.
