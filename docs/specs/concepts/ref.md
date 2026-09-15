# La ref de bilinks

Ninguna rama del proyecto contiene `.bilink/`. Los bilinks viven en `refs/bilink/<branch>`, una ref por rama del proyecto: los de `rc-2.35` están en `refs/bilink/rc-2.35`.

Es lo que todo comando que escribe sobre la ref tiene que cumplir —`init`, `sync`, `track`, `adopt`, `pull`, `relayer`, y también `accept` y `apply`, que commitean como parte de su acto—, y lo que `verify-ref` verifica desde afuera.

## Dónde vive

### Fuera de `refs/heads/`

La ref no es una rama. `git branch -a` no la lista, la UI de la forja tampoco —los listados de ramas muestran `refs/heads/*`— y `git log --branches` la ignora. Es lo que hacen `git notes` con `refs/notes/*` y Gerrit con `refs/changes/*`.

Requiere refspecs explícitos para push y fetch. Los pone `init`; nadie los tipea.

### Qué lleva adentro cada commit

El árbol del proyecto más `.bilink/`. No una rama huérfana con sólo los bilinks: cada commit es un snapshot consistente por construcción, y eso es lo que hace simple el caso remoto: el consumidor trae una sola ref y obtiene las declaraciones del proveedor junto con exactamente el código al que apuntan.

El árbol no se duplica: git comparte los objetos con la rama del proyecto, así que el costo marginal de cada snapshot es la carpeta `.bilink/` y nada más.

## La invariante de fidelidad

### Dos enunciados, verificables sin tree-sitter

1. El árbol de código de todo commit de `refs/bilink/<branch>` es idéntico al del commit del proyecto absorbido más recientemente: el segundo padre del merge más cercano siguiendo primeros padres, o el propio si el commit es ese merge.
2. El commit del proyecto contra el cual `accept` o `apply` calcularon su trabajo tiene que estar absorbido antes de commitear.

El primero es una comparación de tree oids: exacta, y barata porque el merge más cercano casi siempre es el commit mismo o el anterior. El segundo garantiza que el commit absorbido sea el correcto. Y la exigencia de [`accept`](accept.md) —falla sobre un archivo sucio— es lo que impide que el enunciado 2 se cumpla sobre un commit que no contiene lo que se aceptó.

Absorber no es un comportamiento por comando: es una precondición de todo commit sobre la ref. No hay una tabla de quién absorbe y cuándo: hay una condición que se verifica y, si no se cumple, se cumple absorbiendo en un commit propio, inmediatamente antes. Cuando ya se cumple —el proyecto no se movió desde la última absorción— no se absorbe nada, y el enunciado 1 sigue valiendo porque el árbol de código no cambió.

### Un commit hace una cosa

Un commit sobre la ref trae código, o decide, o sincroniza decisiones. Nunca dos de las tres.

Nunca hay un commit que absorba y decida a la vez, y por eso la historia se lee sin abrir un solo archivo:

| Tipo | Padres | Árbol de código | `.bilink/` |
|---|---|---|---|
| 1 · absorción: trae el código del proyecto | dos: la ref y el commit del proyecto | cambia | sin tocar |
| 2 · decisión: `accept`, `apply`, `relayer` | uno | sin cambios | cambia |
| 3 · sincronización: trae bilinks aceptados por otro | dos, los dos de la ref | sin cambios | cambia |

Los tres se distinguen con git a secas, sin leer un bilink: por la cantidad de padres, de dónde vienen, y cuál de los dos árboles se movió.

`sync` no es un tipo propio: es la absorción invocada explícitamente, con la misma forma que la que `accept` dispara cuando le falta.

### El tipo 3 tiene dos casos, y también se distinguen solos

| | De dónde | Los dos lados vienen de… |
|---|---|---|
| 3.a: otra rama, que puede haberse rebaseado sobre la trackeada | `adopt <rama>` | dos absorciones distintas |
| 3.b: la misma rama: alguien tenía una versión anterior y aceptó | `pull <remoto>` | la misma absorción |

El discriminador es de git: se busca hacia atrás la absorción más cercana de cada lado y se comparan. Si es la misma, es 3.b; si son dos, es 3.a.

Y la diferencia importa porque en 3.a los dos lados describen código distinto —cada rama absorbió lo suyo—, así que el árbol de código del resultado tiene que salir del primer padre y no fusionarse. En 3.b los dos lados describen el mismo código, así que no hay nada que elegir.

### 3.b converge por construcción

Dos personas que aceptan el mismo contenido escriben los mismos bytes, porque `link`, `hash` y `hash_ast` direccionan por contenido. El merge de 3.b no tiene qué conflictuar.

Eso vale porque `commit` no está en `accepted`. Es el único campo candidato que nunca converge —el mismo contenido aceptado en dos ramas vive en dos commits distintos—, y meterlo adentro habría hecho que 3.a y 3.b conflictuaran en cada endpoint aceptado de los dos lados, sobre algo que nadie decidió.

### Lo que la invariante no dice

Que los bilinks del commit estén en `OK`. Una versión más fuerte —*"todo `accepted.link` resuelve, contra el árbol de ese commit, a su `accepted.hash`"*— prohibiría que la ref contenga drift, cuando el drift es el estado normal que la herramienta existe para reportar. Con `track` queda evidente: una rama nueva hereda bilinks que describen código de otro commit, y casi seguro arranca con drift. Correcto que lo haga.

## Cómo se arma el commit

### El árbol se construye enumerando, no fusionando

```
read-tree     <commit del proyecto absorbido>       ← el nuevo, si hay que absorber;
update-index  únicamente .bilink/                     el vigente, si ya está absorbido
```

Nada del árbol de trabajo fuera de `.bilink/` entra jamás al commit de la ref. `cache/`, `index/`, `head` y `.bilink-migrate-*` quedan fuera del índice de bilinker, no sólo del índice del proyecto. Y de adentro de `.bilink/` queda afuera el clon de un proveedor —`.bilink/<alias>/`—, que es otro repo entero y no contenido de esta capa.

La lista es la regla, y no hay ningún `.gitignore` detrás. El árbol se construye enumerando: lo que no está en la lista entra, y lo que no tiene que entrar se saca de la lista, no agregando una línea a un archivo versionado. La exclusión del lado del proyecto ya la puso `init` con un solo patrón, y `.bilink/.gitignore` gobierna otra cosa: los derivados, para el repo que todavía no cortó y tiene `.bilink/` en su rama.

Y por eso la fusión de contenido de un `adopt` queda confinada a `.bilink/`: el árbol se construye, no se fusiona. Un `git merge` a secas entre dos refs fusionaría dos árboles de código enteros.

### Las dos verificaciones previas

Corren antes de cada commit, sobre lo que bilinker está por escribir:

1. Disyunción. El árbol del commit del proyecto que se absorbe no contiene `.bilink/`. Si lo contiene, alguien mergeó la ref al proyecto o commiteó bilinks a mano. Se aborta. Es sobre el árbol y no sobre el diff a propósito: el commit que borra `.bilink/` tiene un diff que lo toca y un árbol que no, y es exactamente el commit que hay que poder absorber: el `X` del corte.
2. Fidelidad. El árbol de código del commit nuevo es idéntico al del commit del proyecto absorbido vigente. Comparación de tree oids; si falla, abortar.

Con eso, *"la ref nunca se mergea de vuelta"* deja de ser una regla de buena conducta y pasa a ser una condición chequeada. No es exigible como invariante —nadie puede impedir que alguien mergee— pero sí detectable antes de que contamine nada.

Cómo reparar una disyunción rota, y qué hacer con bilinks sin procedencia, es de la decisión `verificar-ref-ajena`.

## Evolución: merge, y en una sola dirección

### La ref absorbe la rama con un merge, nunca con rebase ni cherry-pick

Un rebase reescribiría la historia que `get --diff` necesita recorrer hacia atrás, y un cherry-pick copia los commits en vez de referenciarlos.

Los merges no conflictúan: la ref no modifica archivos del proyecto —su único diff es agregar `.bilink/`— y el proyecto no toca `.bilink/`. Los dos lados escriben conjuntos disjuntos, y eso se cumple desde el corte, porque la ref nace de un commit del proyecto en el que `.bilink/` ya no existe.

```
rc-2.35:             X ─────── B ─────── C ───────────────────── D ─────── E
                     │                   ╲                       ╲         ╲   ← 2º padre: lo que se absorbe
refs/bilink/rc-2.35: ●0 ───────────────── ●1 ─ ●2 ─ ●3 ─ ●4 ────── ●5 ─ ●6 ─── ●7
                     corte                └ absorción             └ absorción  └ sync
```

| | Padres | Acto | Diff de `.bilink/` contra su 1er padre |
|---|---|---|---|
| `●0` | `X` | el corte | agrega `.bilink/` entero |
| `●1` | `●0`, `C` | absorción de `C` | vacío |
| `●2` | `●1` | `accept 7f3d8e9a.0` | el `accepted` de un endpoint |
| `●3` | `●2` | `accept 7f3d8e9a.1` | el `accepted` del otro |
| `●4` | `●3` | `apply -y` (3 renames) | tres commits, uno por `link` repuntado, acá abreviados; ningún `accepted` tocado → los tres quedan `RELOCATED` |
| `●5` | `●4`, `D` | absorción de `D` | vacío |
| `●6` | `●5` | `accept . --place` (3 endpoints) | tres commits, uno por endpoint, acá abreviados |
| `●7` | `●6`, `E` | `sync` = absorción de `E` | vacío |

Los tres merges —`●1`, `●5`, `●7`— tienen el diff de `.bilink/` vacío, y eso es lo que los identifica: traen código y no deciden nada. Sus hijos son al revés: tocan sólo `.bilink/` y su árbol de código no cambia.

`●4` muestra por qué la fidelidad se enuncia con *"el merge más cercano"*: nadie tocó el proyecto entre `●1` y él, así que no hay absorción nueva y su árbol de código sigue siendo el de `C`.

El corte (`●0`) es el único commit de la ref sin ningún commit del proyecto absorbido por debajo: nace de `X` como padre único, y ahí la fidelidad se lee contra `X` mismo. Es también el único caso en que una decisión puede no tener una absorción arriba.

### La correspondencia con el proyecto es el segundo padre

Y por lo tanto un hecho de git y no una convención de nombres: se recorre con `git log --parents`, y `git branch --contains` y `git merge-base` la responden solas. Un commit de un solo padre la hereda del merge más cercano hacia atrás.

No hace falta ningún identificador propio: el hash del commit ya lo es, y el estado anterior es `refs/bilink/rc-2.35~1`.

### El recorrido se frena al salir de la ref, y el freno es la disyunción

Los commits de la ref llevan `.bilink/` en su árbol y los del proyecto no, así que buscar hacia atrás el commit absorbido se para en el primero que no lo lleva, y ése es la respuesta.

Sin ese freno el corte daría la respuesta equivocada: `●0` tiene un solo padre, `X`, así que el recorrido seguiría hacia atrás por la historia del proyecto y devolvería el segundo padre del primer merge ajeno que encontrara. Con el freno, `●0` contesta `X`, que es exactamente contra lo que su fidelidad se lee.

Es la segunda cosa que la disyunción compra. La primera es detectar que alguien mergeó la ref al proyecto; ésta es poder distinguir los dos lados sin marcarlos.

Y vale para todo recorrido de la ref. `git log --first-parent refs/bilink/<branch>` tampoco se detiene solo: al llegar al corte sigue hacia atrás por la historia del proyecto. Los commits propios de la ref son los que llevan `.bilink/` en su árbol, y ésa es la lista sobre la que se leen el registro de decisiones y los candidatos de `track`. Sin el freno, `track` sobre una rama vieja elige mal de forma sistemática: el commit más viejo del proyecto es ancestro de cualquier rama, así que siempre califica y siempre gana.

Y se lee bien: `git log --first-parent refs/bilink/rc-2.35` muestra sólo la evolución de los bilinks, ocultando la historia del proyecto absorbida.

```
$ git log --first-parent --format='%h %an  %s' refs/bilink/rc-2.35
b1e3f55  Kim     absorb e91f0c4: rc-2.35 al día
9c1f0ab  Ana     accept --place a1b2c3d4.0: scip/link.rs
5d20e81  Ana     accept --place 8e9f0a1b.0: scip/link.rs
3f8b41c  Ana     accept --place 7f3d8e9a.1: scip/link.rs
c7e0d92  Ana     absorb d0b7a12: el rename ya commiteado
4e77d20  Ana     apply 7f3d8e9a.1 3ca90f81…: sciplink.rs → scip/link.rs
77a0c94  Luis    accept 7f3d8e9a.0: spec de check ↔ check_structural
2b1a5f0  Luis    absorb c4e1770: rc-2.35 hasta C
0af3c12  Luis    track rc-2.35: corte 005, los bilinks salen de la rama
```

Ése es el registro de decisiones: quién aceptó qué y cuándo, sin una sola línea del historial del proyecto de por medio. `bilinker log` lo muestra.

### Granularidad: un commit por decisión, no por invocación

`accept <uuid>.0` da un commit; `accept .` sobre veinte endpoints da veinte, encadenados, todos hijos del mismo merge. Vale igual para `apply`, que también escribe decisiones: un `apply -y` que repunta tres `link` escribe tres.

La granularidad sigue al objeto y no al acto, por tres razones. Atribución por decisión: la responsabilidad vive en el commit que escribió el valor, no en el valor, y una firma sobre un commit que aprueba veinte fragmentos dice mucho menos que veinte firmas sobre uno cada una. Varias personas, varios caminos: un mismo capture puede tener N bilinks, y aceptarlos es trabajo de gente distinta en momentos distintos; con el commit como unidad de decisión, cada aprobación es un objeto propio que se lee, se firma y se audita sola. Y hace caro esconder una aprobación masiva: un commit disimula cien decisiones; cien commits firmados las denuncian.

Deshacer una aceptación no necesita `git revert`: es reescribir su `accepted` con los valores anteriores, un commit nuevo, leídos de `refs/bilink/<branch>~n`. La unidad de movimiento es el contenido del archivo, no el commit.

## El mensaje es el comando

### La primera línea empieza con el comando canónico que lo produjo

De un vocabulario cerrado:

```
absorb  <commit-del-proyecto>                   ← tipo 1
track   <rama>                                  ← tipo 1: la ref nace
accept  [--place|--content] <uuid>.<N>          ← tipo 2
apply   <uuid>.<N> <capture-nuevo>              ← tipo 2
relayer <capa>                                  ← tipo 2
adopt   <rama>                                  ← tipo 3.a
pull    <remoto>                                ← tipo 3.b
```

Cada verbo es un comando que existe. No hay verbo para el corte: el comando que lo escribe es `track` en su caso *"no hay de quién heredar"*, y un verbo propio nombraría un comando que nadie puede correr. Los dos nacimientos se distinguen por los padres —el corte tiene uno solo, y no es de la ref— que es como se distingue todo lo demás de un commit de la ref.

Cada comando nombra el objeto sobre el que actuó, y nada más: el endpoint para una decisión, la rama de origen para una sincronización, el commit traído para una absorción, la rama que nace para un `track`. Lo que los padres ya dicen no se repite salvo donde hace legible el log: `absorb` nombra el commit que además es su segundo padre, y eso es deliberado: el registro se lee sin abrir el DAG.

`adopt` no lleva endpoint. Trae todo lo que el vecino decidió entre la base y su tip, en un solo commit y no N, y el conjunto sale del merge a tres puntas entre los dos padres, que ya están en el objeto. `pull` y `adopt` son los dos casos del tipo 3, y llevan verbos distintos porque nombran fuentes distintas: `adopt` una rama vecina, `pull` la copia que el remoto tiene de esta misma ref.

Después del comando puede ir `: ` y prosa libre para quien lee, y el cuerpo es libre. Al final, un trailer obligatorio:

```
accept 7f3d8e9a.0: spec de check ↔ check_structural

Bilinker-Version: 0.4.1
```

Canónico quiere decir derivado del acto, no tipeado por la persona. Un `accept .` de veinte endpoints escribe veinte commits, y cada uno lleva su comando —`accept <uuid>.<N>`—, no `accept .`. Lo que la persona tipeó puede ir como trailer `Invocation:`, que es dato de auditoría y no de verificación.

De ahí sale la propiedad que hace útil el formato: el comando más el árbol del primer padre determinan el árbol resultante. Es la invariante 4 de [la aceptación](accept.md) leída como contrato de reproducción. Quien quiera verificar un commit corre el comando contra el árbol del padre y compara tree oids.

### `Bilinker-Version` es obligatorio, y no alcanza con la versión del formato

La versión del formato está atada a la del crate `bilink-format`, pero `hash`, `hash_ast` y el recorte de bordes viven en `bilinker`: un cambio ahí movería los hashes sin bumpear el formato, y la reproducción de commits viejos empezaría a fallar sin que nada esté mal.

### Un mensaje se parsea, nunca se ejecuta

Es texto que escribe cualquiera, así que un verificador que se lo pase a una shell tiene ejecución remota. El vocabulario es cerrado justamente para eso: se parsea a una forma estructurada, se valida cada argumento contra su tipo —un UUID es un UUID, un índice de endpoint es `0` o `1`—, y el proceso se lanza con argv armado. Un verbo desconocido invalida el mensaje; no es texto libre que se pasa por alto.

Y un comando por commit, que es la misma regla de "Un commit hace una cosa" dicha desde el mensaje. De ahí que `apply` sobre tres endpoints escriba tres commits y no uno.

La gramática la arma y la parsea un mismo módulo, en el mismo lugar: es lo que hace verificable el round-trip. Está aparte de la escritura de la ref a propósito: no toca git y no conoce el repo, es texto contra una forma estructurada. El parser es además la superficie que recibe texto de afuera —un push viene de otra máquina—, y aislarlo es lo que permite decir, y testear, que de ahí no sale nunca una línea de comando: sale un `enum` con el que se arma argv.

### La gramática no es retroactiva

Los commits que ya están en la ref no se pueden reescribir: es append-only. Exigirles la gramática rechazaría la historia entera, así que la regla es de alcance y no de contenido: un verificador valida los commits del push, no la historia alcanzable desde ellos.

Y el discriminador no hace falta inventarlo: la ausencia de `Bilinker-Version` significa *"anterior a la gramática"*, y eso no es un error. No hay commit de corte ni marcador: el trailer se describe solo. Con el trailer puesto, el mensaje tiene que parsear; sin él, no se lo interroga.

### Antes del corte no hay ref, y el commit no ocurre

Un repo que todavía no corrió el corte tiene sus bilinks en la rama del proyecto y ninguna `refs/bilink/*`. Ahí `accept` y `apply` no escriben ningún commit sobre la ref —no hay ninguna— y los cambios quedan en el árbol, visibles con `git status`, para que los commitee quien trabaja.

La existencia de la ref de la rama es lo que enciende el commit del acto. No hace falta un flag ni una versión de formato: el corte es el interruptor, y lo que lo vuelve honesto es que crear la ref sea un acto explícito de `track`.

Eso es lo que permite que el binario nuevo corra sobre repos que todavía no cortaron: si la herramienta se rompe, tiene que quedar con qué diagnosticarla.

## El índice propio

### Bilinker usa su propio `GIT_INDEX_FILE` sobre el mismo árbol de trabajo

Los `.bilink/` están en el árbol de trabajo, no en un worktree aparte: quien usa bilinker o lattice quiere esos archivos a mano. El proyecto los ignora vía `.git/info/exclude`, no vía `.gitignore`, que está versionado y modificarlo tocaría la rama del proyecto.

Ignorados a secas, los cambios que escribe `accept` no aparecerían en ningún `git status`: para el índice del proyecto son archivos ignorados, y la ref donde cuentan no está checkouteada.

Bilinker usa su propio `GIT_INDEX_FILE` contra `refs/bilink/<branch>`. El mismo `.bilink/` queda ignorado por el índice del proyecto y trackeado por el de bilinker, que así recupera `status` y `diff` reales sin ensuciar los del proyecto. Es el patrón conocido de los dotfiles en repo bare.

El índice vive en `.git/bilink/index`: dentro de `.git/`, porque es por clon y no se versiona, y con nombre propio porque no es el índice del proyecto.

### La ref es por repo, y el recorrido se para en su frontera

Un solo `refs/bilink/<branch>` cubre todas las capas de un repo, estén donde estén: el commit lleva el árbol del proyecto entero, así que los `.bilink/` de todas sus capas entran en el mismo snapshot. Es la misma razón por la que la exclusión de `init` es un patrón y no una lista.

Pero un subdirectorio con su propio `.git` es otro repositorio, y sus bilinks son suyos. El recorrido se para ahí.

No es hipotético: en accreta cada subsistema tiene su capa de implementación en un repo propio, gitignoreado por el padre. Sin ese freno, el corte del padre se traga los bilinks de los hijos, y quedan en un snapshot cuyo árbol de código no los contiene, así que ni la disyunción ni la fidelidad hablan de ellos. Los dos chequeos pasarían, y estarían mirando otra cosa.

Es [root.md](root.md), "bilinker siempre opera en el contexto de una sola capa", dicho desde la ref: cada capa puede ser un repositorio git independiente, y bilinker siempre opera en el contexto de una sola. La frontera del repo es la de la ref.

### `check` corre en caliente

Bilinks y código vivo en el mismo directorio, con los cambios sin commitear a la vista, que es lo que [`check`](check.md) exige al comparar contra el árbol de trabajo y no contra HEAD. Si se comparara contra la copia de código de la ref, quien rompe un vínculo no se enteraría hasta que alguien sincronice.

La asimetría local/remoto: localmente el código sale del árbol de trabajo, así que la foto de la ref puede estar atrasada sin afectar un `check`. Remotamente el consumidor sólo tiene la ref, así que esa foto es el código con el que verifica. Por eso la absorción no es un requisito para observar, sino para que el snapshot sea cierto para quien no tiene otra cosa.

## `.bilink/head`: de dónde salió el árbol

### `head` dice a qué rama y a qué commit corresponde el `.bilink/` del árbol

Materializar `.bilink/` en el árbol y excluirlo del índice del proyecto tiene un filo: `git checkout` no lo toca. Cambiar de rama mueve el código y deja los bilinks donde estaban.

`.bilink/head` es un archivo con dos valores, la rama y el commit de `refs/bilink/<branch>` que le corresponde:

```
branch rc-2.35
commit 9c1f0abf3e21d5c4b7a08e6f2d1934ac5b7e0f18
```

Lo escribe tanto la materialización como todo commit sobre la ref: si `accept` avanza la ref, el árbol pasa a corresponder al commit nuevo y `head` tiene que decirlo, o la guarda se dispararía después de cada aceptación.

No se commitea: se suma a `cache/` y `index/` en la lista de lo que queda fuera del índice de bilinker. No es configuración, es estado del árbol de trabajo: exactamente lo que `HEAD` es para git.

Sin punto, aunque `.bilink/` tenga adentro un `.{alias}.toml`. La regla que gobierna ese punto es la de Stratum, donde un dotfile describe a su hermano sin punto; `head` no describe a nadie. Y `.bilink/` ya es un directorio oculto: adentro, el punto no esconde nada.

### La materialización es automática

Cualquier comando compara `head` contra la rama del proyecto, y si no coinciden materializa el `.bilink/` de la ref correcta y sigue. No hay comando de más que tipear, ni pregunta que contestar: no hay nada que decidir.

### La guarda es una sola, la de git

Si el `.bilink/` del árbol difiere del commit que `head` nombra, hay trabajo que no está en ninguna parte —`.bilink/` está fuera del git del proyecto— y materializar lo destruiría. Ahí se para y se avisa, igual que `git checkout` se niega a pisar cambios.

En el flujo diseñado esa ventana no existe: `accept` y `apply` commitean sobre la ref como parte del acto, así que el árbol queda limpio contra el índice propio. Queda abierta sólo para un `apply` que crashea a mitad o una edición a mano, y las dos merecen un humano.

### Por qué un archivo y no inferirlo de la cache

Porque [la cache](cache.md) es un derivado y estar fría es un estado normal, o sea que no puede responder justo cuando más falta hace. Y peor: haría que un derivado autorice sobrescribir la fuente.

Son dos marcadores para dos preguntas distintas, y los dos hacen falta: `head` responde *"¿son éstos los bilinks que corresponden?"* y protege la fuente; el commit que anota `cache/state` responde *"¿estos estados se calcularon sobre estos bilinks?"* y protege el derivado.

### En `HEAD` desacoplado no se materializa nada

A mitad de un rebase con conflictos no hay rama actual contra la cual comparar, y adivinar una sería peor que no hacer nada. Ahí los comandos de lectura corren contra lo que `head` dice, avisando; los que commitean sobre la ref se niegan hasta volver a una rama.

### Tres desajustes, tres arreglos

`head` no se mete con el rebase. Un `git rebase main` estando en `feature/x` te deja en `feature/x`, y no toca `refs/bilink/feature/x`: `head` sigue diciendo `(feature/x, ●a)` y todo coincide. No hay materialización que disparar, porque los bilinks no se movieron: se movió el código debajo.

| Qué se movió | Cómo se detecta | Qué lo arregla |
|---|---|---|
| la rama (`checkout`) | `head` ≠ rama actual | materializar: automático, sin comando |
| el código de la rama (commit, rebase, merge) | el commit absorbido ≠ tip de la rama | `sync`, o el `accept`/`apply` siguiente, que absorbe igual |
| las decisiones del vecino (rebase sobre otra rama) | `status` lo avisa: la ref del vecino avanzó | `adopt <rama>`, con `--dry-run` para verlo antes |

Sólo el primero puede resolverse solo, y por eso es el único que se resuelve solo: los otros dos escriben un commit sobre la ref, y ninguno de los dos es una decisión que la herramienta pueda tomar por su cuenta.

## La ref es protegida

### `refs/bilink/*` sólo avanza, y nadie la escribe a mano

Sus únicos escritores son los comandos de bilinker. Eso no es higiene, es carga estructural: la ref es lo único que conserva los commits del proyecto contra los que se aceptó —los alcanza como segundos padres— y lo único que vuelve verificable una decisión, porque el commit firmado es el artefacto. Reescribirla deja sin baseline y sin atestación a toda aceptación del repo.

Tres niveles, y sólo el último es exigible:

| Nivel | Qué da | Con qué |
|---|---|---|
| el clon local | detección | la disyunción y la fidelidad, chequeadas antes de cada commit |
| el refspec | que no se pise por accidente al traer | el de `init`, sin `+` |
| el servidor | rechazo | un `pre-receive`, y sólo eso |

### La config del servidor no alcanza, y no es una omisión: no aplica

`receive.denyNonFastForwards` y `receive.denyDeletes` son las dos opciones que uno pondría, y no cubren esta ref: git las chequea únicamente sobre `refs/heads/`. Comprobado, con control:

```
$ git -C origin.git config receive.denyDeletes true
$ git push origin ':refs/bilink/main'
 - [deleted]         refs/bilink/main
$ git push origin ':refs/heads/main'
 ! [remote rejected] main (deletion prohibited)
```

Es el mismo precio que estar fuera de `refs/heads/` cobra en la UI de la forja. No es un problema de GitHub y GitLab: la restricción es de git.

De ahí sale algo que cambia la forma de la protección: el `pre-receive` no es la capa cara sobre dos baratas, es la única capa. Que no se borre y que sólo avance son dos filas más de lo que `verify-ref` chequea, donde ya tenían que estar para el caso del que recibe una ref ajena.

### El servidor puede verificarlo sin ejecutar bilinker

Las invariantes de forma de la ref son comparaciones de tree oids, no análisis de contenido. Un `pre-receive` las chequea con git a secas, sin instalar nada:

| Se verifica | Cómo |
|---|---|
| avanza y no se borra | el tip viejo es antepasado del nuevo, y el nuevo no es nulo |
| disyunción | el árbol del commit absorbido no contiene `.bilink/` |
| fidelidad | el árbol de código del commit nuevo es el del absorbido |
| un commit hace una cosa | cae en uno de los tres tipos y no en dos: los padres y cuál de los dos árboles se movió lo deciden |

Ninguna necesita tree-sitter ni abrir un bilink. Lo que el servidor no puede verificar es si lo aceptado es correcto: eso es una decisión humana y no una propiedad del árbol.

### La autorización es la firma, más una regla de una línea

Todo commit sobre la ref tiene que estar firmado por una clave de una allowlist, que `git verify-commit` chequea offline y sin infraestructura.

Del lado de quien escribe, la condición la pone git y no bilinker: se firma si `commit.gpgsign` está puesto, igual que cualquier otro commit del repo.

Y con eso alcanza para que [`agree`](accept.md) deje de ser auto-declarado, sin traducir ningún nombre a ninguna clave: un commit sólo puede agregar a su propio autor a un `agree`. La firma ata el commit a una clave, y con ella al autor que declara; la regla ata los nombres agregados a ese mismo autor. Las dos juntas cierran la cadena: `- ana` sólo puede haberlo escrito un commit firmado cuya autora es Ana. Nadie aprueba en nombre de otro.

Sacar un nombre no está restringido, y no puede estarlo: es lo que hace `adopt` al traer valores distintos, y lo que hace un `accept` cuando los valores cambian y la lista se vacía. Lo que se protege es agregar, que es lo único que afirma algo sobre otra persona.

### Y el prefijo anterior a la gramática pasa una vez

La ref es append-only, así que exigirle la forma a lo que ya está rechazaría el primer push de todo repo que cortó antes de que la gramática existiera. El discriminador es el mismo que para el mensaje —la ausencia de `Bilinker-Version`— y la puerta que eso abre se cierra con una regla de orden: una vez que un commit de la ref lleva el trailer, ninguno de sus descendientes puede no llevarlo.

Un commit sin trailer empujado encima de uno que lo tiene no es historia vieja: es alguien esquivando la verificación.

### La protección de ramas de la forja no alcanza

GitHub y GitLab protegen `refs/heads/*` y `refs/tags/*`; un namespace propio queda afuera de esa UI. Es el precio de estar fuera de `refs/heads/`: se gana invisibilidad y se pierde la protección declarativa, así que hay que ponerla como hook del servidor.

### Localmente no se puede impedir, sólo detectar

Un `git update-ref` en el clon de alguien no lo frena nadie: es su repo. La frontera donde el rechazo es posible es la compartida, y por eso el enunciado fuerte vive en el servidor y no en el cliente.

## Auditoría: contra la ref, no contra la rama

### Todo commit alguna vez absorbido queda alcanzable desde la ref para siempre

La rama del proyecto no está protegida: se rebasea, se force-pushea, se cambia. Pero la ref absorbe los commits del proyecto como segundos padres.

```
git merge-base --is-ancestor <commit> refs/bilink/<branch>   ← precondición
git log <commit>..refs/bilink/<branch> -- <file>              ← la ventana
```

Si aun así no es ancestro —commit nunca absorbido, clon superficial del proveedor— se degrada a *"fuente desconocida"* en vez de devolver un rango inflado.

Y de ahí sale la propiedad: la ref es lo que vuelve inmutable el `commit` de una aceptación. Un rebase de la rama del proyecto saca ese commit de `main`, pero no lo destruye, porque `refs/bilink/<branch>` lo alcanza como segundo padre de una absorción. Protege al `commit` guardado en [la cache](cache.md) y protege también a su re-derivación, que camina la historia del archivo y necesita que esos commits sigan existiendo.

Y es lo que hace que `commit` no necesite estar en `accepted`. La firma de un commit cubre el objeto commit, y el objeto incluye sus padres: el commit del proyecto contra el que se aceptó ya está atestado por la firma de la aceptación, vía el DAG, sin ningún campo en el archivo.

### Autoría, atestación y autorización

Tres cosas distintas que conviene no mezclar:

| | Qué es | Con qué se hace |
|---|---|---|
| Atribución | quién dice que fue | `author`/`committer` del commit de la ref, y `agree` |
| Atestación | prueba de que fue | `git commit -S` · `gpg.format=ssh` · `git verify-commit` |
| Autorización | si tenía derecho a aceptar | la allowlist del servidor, vía `verify-ref` |

Para auditar y revertir alcanza con la atribución; ante alguien que no confía, con la firma. El autor de git es auto-declarado: lo que constituye atestación es la firma, no el campo.

Ésa es la respuesta a por qué la aceptación no necesita un archivo propio para ser revisable: la superficie de revisión es la de bilinker, no la de la forja. La forja no muestra la ref, pero `status`, `diff` y `log` sobre el índice y la ref propios sí, y el artefacto firmable es el commit.

Bendecir lo mismo no diluye la responsabilidad. Dos bilinks pueden tener un `accepted` idéntico, pero los actos que los escribieron son dos commits distintos, cada uno con su autor y su firma. La responsabilidad vive en el commit que escribió el valor, no en el valor.

## `bilinker init`

### Pone a punto el clon, y sin él ningún otro comando corre

Todo lo que la ref necesita son tres cosas puestas en el clon, y ninguna viaja con él: la exclusión en `.git/info/exclude`, el refspec en `.git/config`, y el `.bilink/` materializado en el árbol. Son por clon, no por rama ni por commit, porque las tres viven en `.git/` o fuera de git.

```
bilinker init [--dry-run]
```

| Argumento | Descripción |
|---|---|
| `--dry-run` | Muestra qué haría sin escribir nada. |

No toma path: es por repo, no por capa. Un solo patrón `.bilink/` en el exclude cubre todas las capas de ese repo, estén donde estén.

```
1. .git/info/exclude  ←  .bilink/  y  .bilink-migrate-*
2. .git/config        ←  refspec de refs/bilink/*   → desde acá, git fetch las trae
3. fetch de refs/bilink/*  +  materializar el .bilink/ de la rama actual  +  escribir head
```

### La exclusión va en `.git/info/exclude`, no en `.gitignore`

`.gitignore` está versionado, y agregarlo modificaría la rama del proyecto, justo lo que este diseño evita. `info/exclude` es local, no se commitea y no aparece en ningún MR.

`.bilink-migrate-*` va al lado: esas carpetas son temporales y nunca se commitean. `migrate` ya las escribe ahí al empezar; tenerlas también acá vuelve el clon correcto antes de la primera migración.

Un exclude que ya exista se respeta: se agregan las líneas que falten y no se toca lo demás.

### El refspec se mapea a sí mismo y va sin `+`

```
[remote "origin"]
    fetch = refs/bilink/*:refs/bilink/*
```

Con el refspec puesto, `git fetch` trae las refs de bilinks junto con las ramas. Sin él, una rama al día puede convivir con bilinks viejos, que es la clase de desajuste silencioso que este diseño existe para eliminar.

No compromete la invisibilidad: `refs/bilink/*` sigue sin aparecer en `git branch -a` ni en la forja, porque no está bajo `refs/heads/` ni bajo `refs/remotes/`.

Se mapea a sí mismo, no a `refs/remotes/`. La ref del remoto y la local son la misma cosa: no hay un flujo de trabajo donde alguien tenga bilinks locales adelantados que quiera comparar contra los del remoto sin traerlos.

Y va sin `+`. El `+` de un refspec significa *"actualizá incluso si no es fast-forward"*, y acá es exactamente lo que no se quiere. Como la ref es append-only, en operación normal el fetch es fast-forward y el `+` no aporta nada; lo único que agrega es que, si alguien la reescribió, el fetch pisa la ref local en silencio, y con ella los commits a los que apunta cada aceptación del repo. Sin `+`, ese fetch falla y el problema se ve. Es la mitad del clon de una regla que del lado del servidor es un rechazo.

Si hay más de un remoto, el refspec va en todos. Si ya está, no se duplica.

### El paso 3 materializa y escribe `head`, y la versión llega sola

Se traen las refs, se materializa el `.bilink/` de la rama actual desde `refs/bilink/<branch>`, y se escribe `.bilink/head` con la rama y el commit.

[`.bilink/version`](format-version.md) llega sola: está versionada, así que viaja en el árbol de la ref como cualquier otro archivo de `.bilink/`, y la materialización la escribe con los demás. `init` no la calcula ni la elige: sería la única cosa del directorio que no saliera del commit, y entonces podría discrepar de los archivos que describe.

### El paso 3 no pisa nada

Si hay un `.bilink/` en el árbol y no hay `head`, `init` no puede saber de dónde salió, así que lo deja intacto y se limita a los pasos 1 y 2, y lo dice.

```
$ bilinker init
exclude: + .bilink/  + .bilink-migrate-*
refspec: + refs/bilink/*:refs/bilink/*

.bilink/ presente sin head: no se materializa nada.
  Es lo esperado en el paso 3 del corte 005; en un clon fresco, revisar de
  dónde salió antes de seguir.
```

Es lo que hace que el paso 2 del corte pueda ser un `init` a secas: ahí el `.bilink/` del árbol todavía no está en la ref, y materializar lo borraría. El corte corre el mismo `init` que corre cualquier clon: un camino menos que mantener, y uno que se ejercita todos los días en vez de una sola vez.

### Sin `init`, los comandos fallan; no se auto-configuran

Bilinker arregla solo lo que es suyo, y pide lo que es del repo del usuario. Materializar `.bilink/` es automático porque `.bilink/` le pertenece; escribir en `.git/config` y `.git/info/exclude` es tocar la configuración de otro, y eso merece un acto explícito por más inofensivo que sea. Que el primer `check` configure el repo de callado convertiría un comando de lectura en uno que modifica el entorno.

```
$ bilinker check .
error: el repo no está inicializado para bilinker.
  Correr `bilinker init`.
```

La detección pide las dos piezas que `init` escribe: el exclude, y el refspec en cada remoto que haya. El refspec es la que no puede estar por accidente —un `.bilink/` en el árbol puede venir de antes del corte, y el exclude lo pudo escribir alguien a mano— pero en un repo sin remoto no existe, y pedirla sola lo dejaría sin forma de estar nunca inicializado.

Y sólo se exige donde corresponde: en un repo que ya cortó, que es lo que dice su ledger de migraciones. Antes del corte los bilinks viven en la rama, no hacen falta ni exclude ni refspec, y exigirlos rompería todos los repos que todavía no cortaron, incluida la herramienta con la que se corta. Lo dice el ledger y no el filesystem porque el ledger está commiteado: un clon fresco de un repo que cortó lo sabe antes de tener una sola `refs/bilink/*` local, que es exactamente el caso en el que hay que exigirlo.

### `init` es idempotente

Correrlo dos veces no hace nada la segunda: las líneas ya están, el fetch es fast-forward y no trae nada, y la materialización encuentra el árbol al día. Correrlo después de un `git clone` en una máquina nueva es el caso normal, no una recuperación.

```
$ bilinker init
exclude: + .bilink/  + .bilink-migrate-*
refspec: + refs/bilink/*:refs/bilink/*  (origin)
fetch:   refs/bilink/main  0af3c12..b1e3f55
árbol:   .bilink/ materializado desde refs/bilink/main @ b1e3f55
         5 capa(s), 63 bilink(s), formato 3.0.0
```

Cuando la rama actual no tiene ref:

```
$ bilinker init
exclude: ya estaba
refspec: ya estaba
fetch:   sin cambios

refs/bilink/feature/x no existe.
  Correr `bilinker track feature/x` para crearla.
```

No es un error: una rama nueva todavía no tiene bilinks propios, y quién los hereda es una decisión de `track`, no de `init`.

| Código | Condición |
|---|---|
| 0 | Inicializado, o ya estaba. |
| 1 | No es un repo git; o el fetch falló; o hay un `.bilink/` sucio que la materialización habría pisado. |

`init` no toca ninguna rama del proyecto —escribe en `.git/`, y en el árbol sólo `.bilink/`—, no escribe ningún commit, y no pisa un `.bilink/` sin procedencia.

## `bilinker sync`

### Alinea la ref con la rama, absorbiéndola, y no verifica nada

Cubre el caso en que el proyecto avanzó y nadie aceptó nada. `update` sugeriría que recalcula estados, que es lo que no hace.

```
bilinker sync [--dry-run]
```

No toma rama: opera sobre la rama actual del proyecto y su ref. Cambiar de rama es un `git checkout`, y después de él la materialización es automática.

Un solo commit sobre la ref, con dos padres: el tip de la ref y el tip de la rama del proyecto.

```
1. verificar disyunción sobre el árbol del tip de la rama
2. read-tree    <tip de la rama>
3. update-index únicamente .bilink/
4. commit-tree  -p <tip de la ref> -p <tip de la rama>
5. update-ref   refs/bilink/<branch>
6. escribir .bilink/head
```

El árbol de `.bilink/` no cambia: sale del índice propio, que ya lo tenía. Por eso el diff de `sync` contra su primer padre es vacío, que es lo que lo identifica como el acto que no registra ninguna decisión.

`sync` es la absorción invocada explícitamente, y no un tipo de commit propio. Tiene exactamente la misma forma que la absorción que `accept` y `apply` escriben cuando les falta. Y por eso su mensaje también es el de una absorción: `absorb <commit-del-proyecto>`, no un verbo `sync` propio. El verbo nombra lo que el commit hace, no el comando por el que se pidió; lo que la persona tipeó va como trailer `Invocation:`.

### `sync` no escribe cuando no hay nada que absorber

Si el tip de la rama ya está absorbido, `sync` no escribe ningún commit. Un commit de merge con el mismo segundo padre que el anterior y el mismo `.bilink/` no dice nada que la ref no diga ya.

```
$ bilinker sync
refs/bilink/main ya absorbió main @ 4e77d20 — nada que hacer
```

No es un error, y es el caso más común: `accept` y `apply` absorben como parte de su acto, así que quien trabaja todos los días casi nunca necesita `sync`.

Correr `sync` antes de un `accept` no cambia el resultado: el `accept` habría absorbido igual, en su propio commit. Lo que `sync` compra es poder alinear sin decidir nada, y que la ref del proveedor esté al día para el consumidor remoto, que es quien sí depende de la foto.

### `sync` no recalcula estados ni escribe la cache

No corre tree-sitter, no resuelve captures y no toca `cache/state`. El árbol de código del árbol de trabajo no cambió —`sync` no hace checkout de nada— así que los estados que `check` calculó siguen siendo los mismos. Lo único que cambió es qué commit de la ref los avala, y eso lo anota la cache al ser escrita, no `sync`.

### La disyunción es lo único que puede fallar, y falla ruidosamente

```
$ bilinker sync
error: el árbol de main @ 8c31f0a contiene .bilink/

  Alguien mergeó refs/bilink/main en main, o commiteó bilinks a mano.
  Absorberlo haría que el árbol de la ref contenga dos .bilink/ que git
  fusionaría sin que nadie mire.

  No se escribió nada.
```

### `sync` no publica

Publicar es `push`, y es un comando y no una flag de éste. Sincronizar local y publicar son dos actos, y quien trabaja en una rama propia hace el primero muchas veces antes del segundo. Un `sync` que empujara convertiría un comando local en uno que a veces habla con la red, y *"a veces"* es lo peor que puede ser una operación de red: no se puede correr sin pensar.

```
$ bilinker sync
absorbe:  main  4e77d20..b1e3f55  (3 commits)
commit:   refs/bilink/main  9c1f0ab → 7a2d4e8
diff:     vacío — ninguna decisión registrada
```

| Código | Condición |
|---|---|
| 0 | Absorbido, o ya estaba al día. |
| 1 | La verificación de disyunción falló; o `HEAD` está desacoplado; o la rama no tiene ref. |
| 1 | El `.bilink/` del árbol no corresponde al commit que `head` nombra. |

Si la rama no tiene ref, el arreglo es `track`, no `sync`: crear la ref de una rama es decidir de quién hereda los bilinks, y eso no se adivina. `sync` no habla con la red, es idempotente, y no escribe ningún archivo del árbol de trabajo salvo `.bilink/head`.

## `bilinker track`

### Crea `refs/bilink/<branch>` para una rama que no la tiene

Sin él, empezar a seguir una rama nueva deja todos los endpoints en `PENDING`. La variante `feature/X` de una spec tiene sus bilinks en `refs/bilink/feature/X`, y `track` es lo que contesta qué hereda al abrirse.

```
bilinker track <branch> [--from <rama>]
```

| Argumento | Descripción |
|---|---|
| `<branch>` | La rama del proyecto a trackear. |
| `--from <rama>` | Heredar de la ref de esa rama, en vez de buscarla. |

`--from` nombra la rama del proyecto, no su ref de bilinks: `bilinker track feature/x --from main`. Una sola fuente de verdad, y nadie tipeando namespaces de refs.

```
1. Con --from explícito: se usa y listo.
2. Sin él, buscar entre refs/bilink/* el commit M tal que:
     · M es un commit propio de alguna refs/bilink/<X>, en su cadena de primeros padres
     · su segundo padre P cumple  git merge-base --is-ancestor P <branch>
     · P es el más nuevo de los que cumplen
3. Crear refs/bilink/<branch>:  primer padre M   (hereda los bilinks)
                                segundo padre    tip de <branch>  (absorbe el código)
```

```
main:                  X ─── B ─── C ─── D ─── E
                       │           ╲     ╲     ╲
refs/bilink/main:      ●0 ───────── ●1 ── ●2 ── ●3

feature/x:                               D ─── F ─── G
                                                     ╲
refs/bilink/feature/x:                    ●2 ──────── ●a
```

`●a` es un commit de la misma forma que cualquier otro de una ref, sólo que sus dos padres vienen de lugares distintos: el primero es `●2`, de donde hereda los bilinks; el segundo es `G`, de donde saca el código. Su árbol de código es el de `G` y su `.bilink/` es el de `●2`, así que su diff contra el primer padre es vacío: `track` no decide nada, igual que `sync`.

Y el `.bilink/` heredado sale del árbol de `M`, no del árbol de trabajo: ir por el árbol de trabajo obligaría a materializar antes de saber si el commit se puede escribir.

### `●2` y no `●3`

Los candidatos se miran por su segundo padre: el de `●1` es `C`, el de `●2` es `D`, el de `●3` es `E`. `C` y `D` siguen siendo ancestros de `feature/x`; `E` no, porque la rama se bifurcó antes. `D` es el más nuevo de los que califican, y por eso gana `●2`. Heredar de `●3` traería bilinks que describen código que `feature/x` no tiene.

El test `--is-ancestor` es lo que traduce *"la última versión de los bilinks accesible"* a algo exacto. En el caso común —la rama sale del tip de una rama trackeada y la ref está al día— el `P` más nuevo es el commit base de la rama nueva y la búsqueda termina en la primera iteración.

| Situación | Qué pasa sin el test | Qué hace `track` |
|---|---|---|
| La ref va adelante del punto de fork | hereda bilinks que apuntan a código que la rama no tiene → `UNRESOLVED` y `ALTERED` falsos desde el minuto cero | toma el `M` cuyo `P` es `D` |
| Ningún `P` califica | elegiría algo parecido | lo dice y crea la ref desde cero |
| Califican varias refs | adivina | exige `--from` |

### La búsqueda va de la ref hacia el proyecto, nunca al revés

Ningún commit del proyecto tiene un merge a `refs/bilink/*`: la relación es exactamente la inversa. Buscar en los ancestros de la rama nueva un merge hacia la ref sólo puede encontrar el bug que la verificación de disyunción detecta.

Y la cadena de candidatos son los commits propios de la ref, no todo lo que `git rev-list --first-parent` devuelve: al llegar al corte ese recorrido sigue por la historia del proyecto ("El recorrido se frena al salir de la ref").

### Sin candidato, la ref nace desde cero, y eso es el corte

Cuando ningún `P` califica —la rama sale de antes del corte, o de una línea nunca trackeada— `track` lo dice y crea la ref con el `.bilink/` del árbol de trabajo y el tip de la rama como padre único.

Ése es exactamente el corte 005:

```
1. UN commit que saca .bilink/ del índice de la rama   → X   (pushear antes de seguir)
2. bilinker init  (exclude + refspec)
3. bilinker track <branch>                             → ●0, padre X
4. Ledger: 005
```

El corte no necesita un comando propio ni un `git update-ref` a mano: es el caso *"no hay de quién heredar"* de `track`, que es lo que el corte literalmente es. Un camino menos que mantener, y uno que se ejercita cada vez que alguien abre una rama.

El corte es el único commit de la ref sin ningún commit del proyecto absorbido por debajo: nace de `X` como padre único, y ahí la fidelidad se lee contra `X` mismo.

### Un repo que empieza sale sellado

Si el árbol de trabajo no tiene ningún archivo de bilinks, `track` escribe antes del corte `.bilink/.gitignore` y `.bilink/version`, como cualquier comando que crea un `.bilink/`. Un commit de la ref se reconoce por llevar `.bilink/` en su árbol, y eso es lo que frena el recorrido hasta el commit absorbido. Un corte sin `.bilink/` haría que cada absorción pareciera un commit del proyecto, y ninguna decisión podría escribirse después.

### `track` no es `sync`

`sync` no puede escribir el corte, y es correcto que no pueda: cuando no hay nada que absorber `sync` no escribe ningún commit. En el corte no hay nada que absorber —`X` ya es el tip— y sin embargo hay un commit que escribir, porque el `.bilink/` del árbol todavía no está en ninguna parte. Son dos actos distintos: `sync` alinea una ref que existe, `track` crea la que no.

### Una rama que ya tiene ref es un error

No un no-op silencioso: quien tipea `track` sobre una rama trackeada o se equivocó de rama, o quería `sync`.

```
$ bilinker track main
error: refs/bilink/main ya existe.
  Para ponerla al día con la rama, `bilinker sync`.
```

### El mensaje de `track` es un solo verbo para los dos casos

Porque los dos son el mismo comando:

```
track feature/x: hereda de 9c1f0ab sobre 4e77d20
track main: corte 005, los bilinks pasan a refs/bilink/main
```

El corte no tiene verbo propio y no le hace falta. Lo que lo distingue son los padres —`track` con herencia tiene dos, el corte uno solo y no es de la ref— y eso es lo que la tabla de tipos ya usa para decidir de qué tipo es cualquier commit de la ref.

```
$ bilinker track feature/x
hereda:  9c1f0ab sobre 4e77d20
commit:  refs/bilink/feature/x @ 7a2d4e8
árbol:   64 archivo(s)
```

```
$ bilinker track main
ningún commit de refs/bilink/* califica: la ref nace desde cero.
commit:  refs/bilink/main @ 0af3c12
árbol:   63 archivo(s)
```

```
$ bilinker track feature/x
error: el punto de fork de feature/x es ancestro de main y rc-2.35.
  Elegir con `bilinker track feature/x --from <rama>`.
```

| Código | Condición |
|---|---|
| 0 | Ref creada. |
| 1 | La rama ya tiene ref; o califican varias y falta `--from`; o la rama no existe. |
| 1 | La verificación de disyunción falló sobre el tip de la rama. |

`track` no decide nada —el diff del commit contra su primer padre es vacío—, no absorbe de más —el segundo padre es el tip de la rama nombrada, y nada más—, no pisa trabajo sin commitear, y si el commit no se puede escribir, el `.bilink/` del árbol queda como estaba.

## `bilinker adopt`

### Trae a la ref de esta rama las decisiones que otra rama aceptó

Sin llevarse ninguna de las mías para allá. Es lo que hace falta después de un rebase: rebasear sobre `main` metió el código de `main` en la rama, y si `main` aceptó algo sobre ese código, los bilinks heredados no lo tienen y van a reportar drift que `main` ya resolvió.

```
bilinker adopt <rama> [--dry-run]
```

| Argumento | Descripción |
|---|---|
| `<rama>` | La rama del proyecto de la que traer decisiones. `origin/main` y `main` son lo mismo. |
| `--dry-run` | Calcula y reporta exactamente lo mismo, sin escribir un solo archivo. |

Se nombra la rama del proyecto, no su ref de bilinks, y la traducción a `refs/bilink/main` la hace la herramienta.

### `adopt` no se llama `merge` a propósito

En este diseño *merge* ya nombra una cosa muy precisa y estructural: un commit de la ref que absorbe un commit del proyecto como segundo padre. `adopt` dice lo que pasa y es asimétrico, que es la verdad: las decisiones del vecino entran acá, y ninguna de las mías va para allá.

### `adopt` son dos commits, y por qué

```
refs/bilink/main:           ●2 ── ●3 ─────────────────────╮
                                                          │    2º padre de ●c: trae ●2..●3,
feature/x (rebaseada):            E ─── F' ─── G'         │    las decisiones de main
                                                 ╲        │
refs/bilink/feature/x:      ●a ────────────────── ●b ──── ●c
```

`●b` absorbe `G'`. No tiene nada de especial: es la absorción de siempre. `adopt` escribe un commit sobre la ref, así que la precondición de fidelidad lo obliga a absorber `G'` antes. Es obligatorio en cualquier caso, porque si no la ref sigue afirmando que su código es `G`, un commit que el rebase abandonó.

`●c` trae `●2..●3`: las decisiones del vecino, como segundo padre.

Dos commits y no uno de tres padres. Con `(●a, G', ●3)` en un solo commit, `--first-parent` mostraría una línea para dos cosas distintas, y la invariante de fidelidad necesitaría una regla acompañante para el tercer padre.

Quien sólo quiere ponerse al día sin traer nada del vecino corre `sync`, que escribe `●b` y para ahí.

### La base del merge a tres puntas sale gratis

Es `●2`, sin que nadie la calcule: es la base de merge real entre `●a` y `●3`, porque `track` puso `●2` como primer padre de `●a` en vez de copiar archivos. Ésa es la razón de fondo de la forma que `track` tiene, y recién acá se cobra.

### Qué compara `adopt`

`accepted` son campos con nombre, por endpoint, así que el merge a tres puntas los compara de a uno:

```
$ bilinker adopt origin/main --dry-run
base ●2 · 4 aceptaciones de main en ●2..●3

entra limpio     7f3d8e9a.0   contenido    Luis
                 a3f9c821.1   ubicación    Ana
ya coincidía     c1a2b3c4.0   contenido    — mismo valor de los dos lados
conflicto        d5e6f7a8.0   contenido    main 838ea0a4…  ·  acá 9211a4f3…
```

| Fila | Qué pasó | Qué se escribe |
|---|---|---|
| entra limpio | el vecino cambió el campo y acá nadie lo tocó desde la base | el valor del vecino |
| ya coincidía | los dos lados escribieron el mismo valor | nada: ya está |
| conflicto | los dos lados lo cambiaron, a valores distintos | las dos entradas, y el endpoint queda `CONSENSUS_DIVERGED` |
| *(sin fila)* | acá se cambió y el vecino no | nada: mis decisiones no se pisan |

Que la fila *"ya coincidía"* exista es la convergencia que el direccionamiento por contenido hace posible: dos personas que aceptan el mismo contenido en HEADs distintos escriben los mismos valores. Por eso el caso común de un rebase no conflictúa nada.

Y la fila que no existe es la que dice que `adopt` es asimétrico: un campo que sólo yo cambié se queda como está, y no viaja para el otro lado.

### La declaración se compara igual que la decisión

Cada endpoint tiene dos escritores: `apply` escribe la declaración —`link` y `n`— y `accept` escribe `accepted` ([bilink.md](bilink.md)). Una decisión se toma sobre una declaración, así que `adopt` compara las dos, con las mismas filas:

| Campo | Dimensión en el reporte |
|---|---|
| `link` | `declaración` |
| `n` | `vecindario` |
| `accepted[].link` | `ubicación` |
| `accepted[].hash` | `contenido` |
| `accepted[].agree` | `aprobadores` |

Un conflicto de declaración no se une: un endpoint tiene un solo `link` y un solo `n`. Frena el comando como cualquier conflicto, y se resuelve de un lado con `apply` o `recapture`.

Sin la declaración, una decisión del vecino llega sobre la declaración de acá: el endpoint queda `RELOCATED` por una ubicación que el vecino ya había aprobado.

### Un bilink que sólo tiene el vecino entra entero

Un bilink que no está en la base ni acá, y sí en el vecino, entra con el archivo del vecino, `kind`, `name` y `as` incluidos. Es "entra limpio" sobre el archivo en vez de sobre un campo: de este lado nadie decidió nada sobre él.

```
entra nuevo      035496d1
```

`kind`, `name` y `as` son inertes, y sobre un bilink que ya existe de los dos lados no se comparan.

### Los captures entran por unión, y nunca conflictúan

Un capture es inmutable y su nombre es el hash de su ubicación ([capture.md](capture.md)): dos con el mismo nombre son el mismo archivo. Todo capture del vecino que no está acá entra con el commit de `adopt`. Uno que después no referencia nadie lo saca `capture prune`, como siempre.

Los captures no cuentan como algo que adoptar: si lo único que el vecino tiene de más son captures, no hay nada que adoptar.

### `adopt` no borra

Un bilink que está en la base y no está en el vecino se reporta y se queda. `adopt` no decide nada, y un bilink borrado se lleva su última aceptación de la ref. Quien lo quiere afuera corre `bilinker remove`.

```
borrado allá     a407ca4c    se queda: `bilinker remove a407ca4c` para sacarlo
```

Lo mismo con un capture: `adopt` sólo agrega.

### La divergencia se une, y deja de bloquear

Con [`accepted` como lista](bilink.md), un conflicto se une: las dos entradas quedan, el endpoint pasa a `CONSENSUS_DIVERGED`, y el resto del `adopt` sigue. Es la misma resolución que este comando usa para `agree`: unión, campo por campo, sin preguntarle a nadie.

Con esto el merge es total: todo campo o se mergea o diverge visiblemente, y nada se descarta ni bloquea. Un conflicto no es una razón para no escribir: es una razón para escribir las dos.

Unir no aprueba nada: las dos entradas ya estaban firmadas por quien las escribió, cada una en su rama. Lo que `adopt` hace es traerlas juntas, y el estado dice que falta una decisión, que sigue siendo de una persona mirando con `accept`. `adopt` compone, `accept` decide.

### `adopt --dry-run` no toca la red

Es lo que hace que reporte exactamente lo mismo que el comando real: si `adopt` fetcheara, el dry-run reportaría sobre otros datos que la corrida de verdad. La ref del vecino se trae con `git fetch`, que `init` dejó configurado, y `adopt` opera sobre lo que ya está.

### Ver qué decidió el vecino antes de traerlo

No necesita un verbo propio. Son dos preguntas y las dos ya tienen respuesta: qué actos hubo del otro lado es `bilinker log --first-parent <rama> ^<mi-rama>`, el registro de decisiones acotado al rango que falta; qué le harían a mis bilinks es `bilinker adopt <rama> --dry-run`.

### Cuándo no hay nada que adoptar

Si la ref del vecino no avanzó desde la base, `adopt` no escribe ningún commit y lo dice. `status` es lo que avisa cuando sí avanzó.

```
$ bilinker adopt origin/main
refs/bilink/main no avanzó desde ●2 — nada que adoptar
```

```
$ bilinker adopt origin/main
base ●2 · 4 aceptaciones de main en ●2..●3

entra limpio     7f3d8e9a.0   contenido    Luis
                 a3f9c821.1   ubicación    Ana
ya coincidía     c1a2b3c4.0   contenido

absorbe:  feature/x → 8f1a2b3   (●b)
commit:   refs/bilink/feature/x  ●b → ●c   (2 endpoints)
```

| Código | Condición |
|---|---|
| 0 | Adoptado, o no había nada que adoptar. |
| 1 | La rama nombrada no tiene ref; o `HEAD` está desacoplado; o la disyunción falló. |

`adopt` es asimétrico, no pisa decisiones propias, y `--dry-run` no escribe y no habla con la red.

## `bilinker pull`

### Trae las decisiones que otro aceptó en la misma rama y las une con las propias

Es el caso 3.b de la taxonomía: sincronización de decisiones donde los dos lados cuelgan de la misma absorción. `adopt` cubre el 3.a y no aplica acá, porque no hay otra rama que nombrar.

```
bilinker pull [<remoto>] [--dry-run]
```

| Argumento | Descripción |
|---|---|
| `<remoto>` | De cuál traer. Default: el único que haya, o `origin`. |
| `--dry-run` | Muestra qué entraría sin escribir nada. |

### `pull` es un verbo propio y no `adopt` sin argumento

`adopt <rama>` nombra la rama de la que se trae, y acá no hay ninguna: la fuente es la copia que el remoto tiene de esta misma ref. Un `adopt` sin argumento sería el mismo comando significando algo distinto según le pasen o no un nombre.

Y `pull` es el nombre que ya tiene el acto: es lo que git llama traer del remoto y unir, y es el contrapié exacto del `push` que fue rechazado.

### `push` no lo hace solo

Un `push` que sincroniza al ser rechazado es cómodo y esconde una decisión: unir dos historias de aceptaciones puede tener conflicto, y resolverlo es mirar dos decisiones humanas incompatibles. Así que `push` reporta y nombra el comando que corresponde.

### Un non-fast-forward tiene dos causas, y sólo una es un problema

| Causa | Cómo se distingue | Qué corresponde |
|---|---|---|
| las dos partes agregaron | los dos tips descienden de una base de merge común | `bilinker pull` |
| alguien reescribió | no hay base de merge, o el tip viejo no es alcanzable | mirar, no forzar |

`git merge-base` las separa, así que no hace falta adivinar, y confundirlas es lo peor que puede hacer el mensaje, porque manda a mirar un incidente donde no hubo ninguno.

### El commit de `pull` tiene dos padres, los dos de la ref

El tip propio y el que trajo el fetch.

```
refs/bilink/main (remoto):  ●1 ─── ●2 ──────╮   2º padre: lo que aceptó el otro
                            │               │
refs/bilink/main (local):   ●1 ─── ●3 ───── ●4
                            └ la misma absorción arriba de los dos
```

El árbol de código no se elige. Los dos lados cuelgan de la misma absorción, así que describen el mismo código: sale del primer padre y no se fusiona. Y como la fusión queda confinada a `.bilink/`, el commit no toca ningún archivo del proyecto.

### Qué es conflicto en `pull` y qué no

El merge es a tres puntas y campo por campo, el mismo de `adopt`:

| | Qué pasa |
|---|---|
| endpoints distintos | unión, sin conflicto: cada uno decidió sobre algo que el otro no tocó |
| el mismo endpoint, `accepted` idéntico | ya coincidía: los valores direccionan por contenido |
| el mismo endpoint, `accepted` distinto | conflicto: dos decisiones humanas incompatibles |
| el mismo endpoint, mismos valores y `agree` distinto | unión, y el resultado dice algo verdadero que antes no se podía decir: los dos aprobaron |

La última fila es la que hace que el caso más frecuente se cierre sin criterio humano. Si Ana y Luis aceptaron lo mismo, los valores coinciden byte a byte, y lo único que difiere es [`agree`](accept.md), cuya reconciliación es una regla, no una decisión.

Vale igual para la declaración, para un bilink que sólo tiene el remoto, para los captures y para lo que el remoto borró: las reglas de `adopt`, desde § "La declaración se compara igual que la decisión" hasta § "`adopt` no borra".

Con un conflicto no se escribe nada, ni siquiera el fetch se deshace: resolver un `accepted` en conflicto es elegir una de dos decisiones, y eso es `accept`, con una persona mirando.

### El fetch de `pull` va a un namespace aparte

```
refs/bilink-remote/<remoto>/<branch>
```

No a `refs/bilink/<branch>`, que es la ref local y es justo la que no se quiere pisar: el refspec sin `+` de `init` existe para que ese fetch falle en vez de pisarla. Ahí sí se trae forzando, y no hay nada que proteger: es una copia de lectura del remoto, se descarta y se vuelve a traer.

Y el fetch va con `--refmap=`. Sin él, `git fetch <remoto> <refspec>` aplica además los refspecs configurados, y el de `init` va sin `+` justo para fallar cuando el remoto divergió: el fetch de `pull` fallaría exactamente en el único caso en que `pull` existe. El refmap vacío apaga esa parte y deja sólo el refspec que se pidió.

```
$ bilinker pull
base ●1 · 3 aceptaciones de origin en ●1..●2

  entra limpio  7f3d8e9a.0  contenido   ← sólo el otro lo tocó
  entra limpio  3a4b5c6d.1  ubicación
  ya coincidía  8e9f0a1b.0  contenido   ← los dos aceptaron lo mismo
                8e9f0a1b.0  agree: ana, luis

commit:  refs/bilink/main  ●3 → ●4   (2 endpoints)
```

```
$ bilinker pull
base ●1 · 3 aceptaciones de origin en ●1..●2

  conflicto     7f3d8e9a.0  contenido
                  acá:   c00e0760…
                  allá:  40acd80f…

no se escribió nada. Resolver aceptando uno de los dos: `bilinker accept 7f3d8e9a.0`
```

| Código | Condición |
|---|---|
| 0 | Unido, o no había nada que traer. |
| 1 | Hubo conflicto, y no se escribió nada. |
| 1 | No hay remoto, o hay varios y ninguno es `origin`. |

Ninguna aceptación se pierde: los dos commits siguen alcanzables desde el resultado, porque son sus dos padres. El árbol de código no cambia. Todo o nada: con un conflicto no se escribe ningún commit. La ref del remoto se trae a `refs/bilink-remote/`, nunca sobre la local.

## `bilinker push`

### Publica `refs/bilink/<branch>` en el remoto

```
bilinker push [<branch>] [--remote <nombre>]
```

| Argumento | Descripción |
|---|---|
| `<branch>` | Rama cuya ref publicar. Default: la rama actual. |
| `--remote` | A cuál publicar. Default: el único que haya, o `origin`. |

### Ninguna interacción con `refs/bilink/*` se hace tipeando git

La ref vive fuera de `refs/heads/`, así que `git push` a secas no la empuja: hay que nombrarla con un refspec. Y hacer que alguien tipee un refspec es exactamente lo que la ref evita. Escribir `refs/bilink/main:refs/bilink/main` una sola vez ya es una fuga del namespace hacia afuera; a la segunda ya es una convención que alguien copia mal, con la rama de otro adentro.

El refspec lo arma bilinker, y el del push va con `+`, a diferencia del de fetch. No es opcional: un clon superficial no tiene la historia para probar que el tip nuevo desciende del viejo, y sin el `+` git rechaza como non-fast-forward un avance legítimo.

Y la diferencia entre los dos es quién te protege:

| | Qué saltea el `+` | Quién verifica igual |
|---|---|---|
| fetch | la verificación sobre tu ref local | nadie |
| push | la verificación del cliente | el servidor, con `verify-ref` |

El `+` del push no es un permiso: es sacarle al cliente una verificación que el servidor hace mejor.

### `push` publica la ref, no la rama

Qué commits de tu proyecto salen a la luz es una decisión tuya, y la tomás con `git push` cuando quieras. `bilinker push` no la toma por vos. Son dos cosas que a veces van juntas y no siempre: se puede publicar la ref de una rama que ya está pusheada, y se puede pushear la rama sin publicar todavía las decisiones.

### Con varios remotos

Con uno solo, se usa ése. Con varios, gana `origin` si está; si no, se pide elegir:

```
$ bilinker push
error: hay más de un remoto (upstream, fork) y ninguno es `origin`.
  Elegir con `bilinker push --remote <nombre>`.
```

Adivinar sería adivinar a quién le publicás, que es la clase de cosa que no se adivina.

### Un rechazo no se fuerza, y tiene dos causas

Decir que un non-fast-forward significa que alguien reescribió la ref es falso en el caso más común: dos personas que aceptan en la misma rama los dos agregaron, y nadie reescribió nada. `git merge-base` las separa, así que `push` no adivina: dice cuál de las dos fue.

```
$ bilinker push
error: origin tiene refs/bilink/main adelantada, y las dos historias agregaron.
  Nadie reescribió nada: base de merge en 0af3c12.
  Unir con `bilinker pull`.
```

Ninguna de las dos se resuelve con `--force`, y por eso no existe.

```
$ bilinker push
publicado: refs/bilink/main @ b1e3f55 → origin
```

```
$ bilinker push
refs/bilink/main ya estaba en origin @ b1e3f55
```

| Código | Condición |
|---|---|
| 0 | Publicado, o el remoto ya lo tenía. |
| 1 | La rama no tiene ref; o no hay remoto; o hay varios y ninguno es `origin`. |
| 1 | El remoto rechazó el push. |

Nadie tipea un refspec, `push` publica la ref y nada más, es idempotente, no fuerza, y no sincroniza.

## `bilinker verify-ref`

### Verifica que una `refs/bilink/*` tenga la forma que la ref promete

Es la misma verificación en dos lugares que no se parecen: del lado del servidor, donde puede rechazar un push, y del lado del que recibe una ref ajena, donde puede avisar antes de calcular drift contra un árbol fabricado.

No mira si los bilinks están en `OK`. Una ref con drift es normal, y exigir `OK` haría imposible `track`. Lo que se verifica es la forma, nunca el contenido de una decisión.

```
bilinker verify-ref [<rango>] [--signers <archivo>] [--stdin]
```

| Argumento | Descripción |
|---|---|
| `<rango>` | `<viejo>..<nuevo>`, o un nombre de ref, y entonces son sus commits propios, del corte para acá. Sin argumento, la ref de la rama actual. |
| `--signers` | El archivo de firmantes autorizados. Sin él, la firma no se verifica y se dice. |
| `--stdin` | Lee `<viejo> <nuevo> <ref>` por línea: el protocolo de un `pre-receive`. |

### Qué verifica `verify-ref`, y con qué

Nada de esto necesita tree-sitter ni resolver una query. Son comparaciones de tree oids, parseo de YAML, y hashes, que es lo que permite correrlo en un servidor que no adoptó bilinker.

Del rango:

| | Por qué |
|---|---|
| la ref no se borra | sin esto, *"sólo avanza"* se esquiva borrándola y empujándola de nuevo |
| el tip viejo es ancestro del nuevo | la ref es append-only; un no-fast-forward es una reescritura |

Los dos los verifica este comando y nadie más: git no chequea `receive.denyNonFastForwards` ni `receive.denyDeletes` fuera de `refs/heads/`.

De cada commit:

| | Con qué |
|---|---|
| el mensaje parsea contra la gramática | el vocabulario cerrado |
| cae en uno de los tres tipos, y no en dos | los padres, y cuál de los dos árboles se movió |
| disyunción: lo absorbido no trae `.bilink/` | `ls-tree` del segundo padre |
| fidelidad: el árbol de código es el del absorbido | `diff-tree` |
| cada archivo que toca valida contra el formato | el esquema, con campos desconocidos rechazados |
| el nombre de un capture es `sha256(file \0 query \0)[..32]` | el id del capture recalculado |
| un capture sólo se agrega, nunca se modifica ni se borra | el diff del commit |
| el formato declarado no es más nuevo que el conocido | `.bilink/version` |
| a `agree` sólo se agrega el autor del commit | el diff de los dos `accepted` |
| está firmado por una clave de la allowlist | `git verify-commit` |

### `agree` sólo se agrega a sí mismo

Es la fila que convierte `agree` de atribución en atestación, y la que hace que no haga falta ningún mapeo de nombres a claves. La firma ata el commit a una clave de la allowlist, y con ella al autor que el commit declara; el hook exige que los nombres que ese commit agregó a algún `agree` sean exactamente el autor del commit. Sacar un nombre está permitido y no necesita ser el propio: lo que se protege es agregar.

### La gramática no se aplica hacia atrás, y tampoco se puede volver

Un commit sin `Bilinker-Version` es anterior a la gramática, y su forma no se verifica. Pero eso deja una puerta, y se cierra con la regla de orden de "Y el prefijo anterior a la gramática pasa una vez": un recorrido del rango, oldest-first, que deja el prefijo viejo pasar exactamente una vez. De la misma regla sale que la firma tampoco se le exige al prefijo.

### El `pre-receive` es la única capa

No hay una capa de config debajo: las dos opciones que uno pondría no aplican fuera de `refs/heads/`. Ponerlas no hace daño y no hace nada; lo que protege es el hook.

```sh
#!/bin/sh
exec bilinker verify-ref --stdin --signers /etc/bilinker/allowed-signers
```

El hook recibe `<viejo> <nuevo> <ref>` por línea y sale distinto de cero para rechazar el push entero. Las refs que no son `refs/bilink/*` se ignoran: este hook no opina sobre las ramas del proyecto.

Un servidor que no quiera instalar el binario puede implementar las mismas filas desde [el esquema publicado](format-version.md): ninguna necesita nada que el esquema no describa.

Que `accepted.hash` sea de verdad el hash del fragmento sí necesita resolver la query, así que no es de acá: es el replay en CI de la decisión `verificar-ref-ajena`.

### Los firmantes son el formato de `allowed_signers` de ssh

Es el que git ya consume por `gpg.ssh.allowedSignersFile`:

```
ana@example.com ssh-ed25519 AAAAC3Nza...
pablo@example.com ssh-ed25519 AAAAC3Nza...
```

No se inventa un formato: git ya sabe leerlo, y una allowlist propia sería una tercera lista de personas en un proyecto que ya tiene dos. Vive en el servidor, no en la ref: una allowlist versionada la edita quien pushea, que es exactamente quien no debería poder ampliarla.

Y bilinker firma lo que escribe, si el repo está configurado para firmar: con `commit.gpgsign`, la misma opción con la que se firma cualquier otro commit. Hizo falta decirlo porque `git commit-tree` —con el que se arma todo commit de la ref— no la mira, a diferencia de `git commit`: sin pasarle `-S` los commits salen sin firmar, y la allowlist se quedaría sin nada que verificar.

Sin `--signers`, la verificación de firma no corre y la salida lo dice: *"sin allowlist: la firma no se verificó"*. Es la diferencia entre *"verifiqué y está bien"* y *"no verifiqué"*, y confundirlas sería el peor resultado posible.

```
$ bilinker verify-ref refs/bilink/main
refs/bilink/main  47 commit(s)

  ✓  38  con la gramática, firmados
  ·   9  anteriores a la gramática — forma no verificada

ok
```

```
$ bilinker verify-ref refs/bilink/main --signers ./allowed-signers
refs/bilink/main  47 commit(s)

  ✗  3f8b41c  absorbe y decide a la vez: el diff de .bilink/ de una absorción es vacío
  ✗  9c1f0ab  agrega `- ana` a 7f3d8e9a.0 y el autor es pablo
  ✗  b1e3f55  sin firma de la allowlist

3 de 47 rechazados
```

| Código | Condición |
|---|---|
| 0 | Todo lo verificable verifica. |
| 1 | Algún commit no cumple. |
| 2 | El rango no se puede leer: la ref no existe, el commit no está. |

`verify-ref` no escribe nada, ni siquiera cache; no resuelve ninguna query ni carga ninguna gramática; un commit anterior a la gramática no se rechaza por su forma, y ninguno de sus descendientes puede serlo; sin allowlist, la firma no se verifica y se dice.

## `bilinker history`

### Contesta qué le pasó a este bilink

Quién aceptó qué, cuándo, contra qué código, y por qué ubicaciones fue pasando. Los demás comandos miran el presente; éste mira la ref, que es donde vive el registro de decisiones. No persiste nada nuevo: arma una vista.

```
bilinker history <uuid>[.<N>] [--format json]
```

| Argumento | Descripción |
|---|---|
| `<uuid>` | Todos los actos sobre ese bilink. Acepta un prefijo. |
| `<uuid>.<N>` | Filtra a un endpoint. |
| `--format json` | Para un consumidor que no es una persona, que es el principal. |

### Los datos de `history` son de git, no del mensaje

Un solo query da la historia:

```
git log --first-parent refs/bilink/<rama> -- .bilink/<uuid>.yaml
```

Cada commit ahí es un acto sobre ese bilink. Lo demás sale del DAG y del diff:

| | De dónde |
|---|---|
| commit de la ref, autor, fecha | el commit |
| tipo: absorción · decisión · sincronización · corte | la taxonomía: los padres y qué árbol se movió |
| el comando canónico | el mensaje |
| el commit del proyecto contra el que se calculó | la absorción más cercana, su 2º padre |
| qué cambió: el endpoint, y el antes/después de `link`, `hash`, `hash_ast`, `agree` | el diff del YAML |
| para un cambio de `link`: los dos captures con su `{file, query}` | los blobs de ese commit |

El comando canónico le da a cada acto su nombre sin heurísticas, pero todo lo demás es derivable sin él, y por eso la vista sirve sobre la historia que ya existe.

### La historia de un capture es la secuencia de `link`

Un capture es inmutable: su id es el hash de su ubicación, así que no tiene historia propia. La que hay es la de los `link` que fueron apuntando a uno y después a otro, y cada cambio de esa secuencia es un `apply`. Con `link` y `accepted.link` los dos presentes se leen las dos dimensiones: cuándo se propuso una ubicación, y cuándo se aprobó.

Y un capture que `prune` borró se sigue leyendo: todo commit que tenía ese capture lo sigue teniendo, así que se lee del árbol de ese commit aunque ya no esté en el del tip. Sin la ref, `capture prune` sería destructivo para la arqueología; con ella, sólo saca del presente lo que nadie referencia.

### `history` degrada por acto, nunca por corrida

Un acto anterior a la gramática —sin `Bilinker-Version`— no tiene comando canónico que leer. Se reporta con todo lo que sí es derivable de git: autor, fecha, tipo por los padres, y el diff del YAML. El comando queda `desconocido`, y nunca se adivina del texto libre: un mensaje viejo que empieza con `accept` no es un `accept <uuid>.<N>`, y tratarlo como si lo fuera sería fabricar precisión.

Un repo que todavía no cortó a la ref no tiene registro de decisiones: los bilinks viven en la rama del proyecto y su historia es la de esa rama. Se muestra, diciendo que es eso —*"sin ref: la historia sale de la rama"*—, porque callar la diferencia haría parecer completa una vista que no lo es.

N commits firmados sobre el mismo valor son N personas de acuerdo, y eso se lee del log sin ningún campo. `agree` lo dice además en el artefacto; `history` muestra los dos: quién lo escribió, y qué decía la lista en cada momento.

```
$ bilinker history 7f3d8e9a
7f3d8e9a-…  docs/spec.md ↔ src/Service.java

  9c1f0ab  Ana    2026-08-31  decisión       accept --place 7f3d8e9a.0
           contra e91f0c4
           .0  link       3ca90f81… → 7d21b0ae…
               agree      —         → ana

  4e77d20  Ana    2026-08-31  decisión       apply 7f3d8e9a.0 7d21b0ae…
           .0  link       3ca90f81… → 7d21b0ae…
               3ca90f81  docs/spec.md         (section (atx_heading …
               7d21b0ae  docs/renombrada.md   (section (atx_heading …

  77a0c94  Luis   2026-08-30  decisión       accept 7f3d8e9a.0
           contra c4e1770
           .0  hash       —         → c00e0760…
               agree      —         → luis

  0af3c12  Luis   2026-08-29  corte          (anterior a la gramática)
           .0  el bilink aparece
```

Con `--format json`, un array de actos con los mismos campos y sin abreviar.

| Código | Condición |
|---|---|
| 0 | Se listó la historia, aunque esté vacía. |
| 1 | El uuid no existe, o es ambiguo. |

La vista completa —decisiones más comentarios más contexto del grafo— es de impact; acá está la primitiva.

## `bilinker relayer`

### Mueve los bilinks de una capa a la de arriba

Existe por un modo de falla concreto: un `.bilink/` fabrica una raíz de capa, porque es uno de los marcadores con los que bilinker [resuelve la raíz](root.md). Si queda en un directorio que stratum no declara como capa, las dos herramientas discrepan sobre dónde termina una, y el `check` de la capa de arriba deja de ver esos bilinks sin decir nada.

```
bilinker relayer <capa> [--dry-run]
```

| Argumento | Descripción |
|---|---|
| `<capa>` | La capa a vaciar, relativa a la actual. |
| `--dry-run` | Muestra qué movería sin escribir nada. |

### Lo que se mueve es la ubicación, nunca el contenido

El id de un capture es `sha256(file \0 query \0)`, así que prefijar el `file` le cambia el id. Los `hash` no se tocan. De ahí que ningún endpoint pase a `ALTERED`: el fragmento no se movió, y lo aprobado sobre él sigue coincidiendo. Lo que cambia es desde qué capa se lo nombra.

Es lo mismo que hacen `apply` y `accept --place` juntos —corregir una ubicación y aprobarla— pero entre capas, que es lo que ninguno de los dos sabe hacer.

| | Qué le pasa |
|---|---|
| cada capture | se reacuña con el `file` prefijado, y cambia de id |
| `link` y `accepted.link` de un endpoint `capture` | el id nuevo |
| `link` de un endpoint `path` relativo a la capa que se vacía | gana el prefijo: `>impl` pasa a `<capa>>impl` |
| `accepted.link` de un endpoint `path` | sólo si el id está en el mapa: es una copia opaca del vecino, y el capture del vecino no se movió |
| los bilinks de las capas que cuelgan de la que se vacía | su `accepted.link` copiaba un id que cambió |
| `hash` y `hash_ast` | nada |

El `path <` de las capas de abajo no se toca, y es lo que hace que la migración sea de un solo lado: al desaparecer el `.bilink/`, ese `<` pasa a resolver a la capa de arriba por sí solo, porque el marcador que lo detenía era ese mismo directorio.

### `relayer` es una decisión, y tiene verbo propio

Su commit sobre la ref es del tipo decisión —un padre, sólo `.bilink/` cambia— y su comando canónico es `relayer <capa>`. Mover bilinks entre capas no es ninguno de los otros actos: `apply` repunta un endpoint a otro fragmento; acá el fragmento es el mismo.

```
$ bilinker relayer subsystems/stratum
subsystems/stratum: 9 capture(s) reacuñados, 10 bilink(s) movidos, 10 vecino(s) con el id actualizado
commit:  refs/bilink/… @ f2a6ca1
```

| Código | Condición |
|---|---|
| 0 | Movido. |
| 1 | La capa no tiene `.bilink/` propio, o es la capa de destino. |

Ningún `hash` ni `hash_ast` cambia; ningún endpoint pasa a `ALTERED`, `UNRESOLVED` ni `RELOCATED`; el `.bilink/` de la capa vaciada se borra, porque si quedara la capa seguiría fabricada; y es todo o nada: los captures se reacuñan en memoria y se escribe recién al final.

`relayer` no decide si una capa debería serlo. Eso lo dice stratum, y bilinker no lo consulta. Este comando arregla el caso; no lo detecta.

## `bilinker log` y `bilinker diff`

### `bilinker log` es el registro de decisiones: los commits propios de la ref

Lista los commits propios de `refs/bilink/<branch>` —`accept`, `apply`, `relayer`, `track`, `adopt`, `absorb`— con su autor y su mensaje, y ninguno del historial del proyecto: es la respuesta a quién aceptó qué y cuándo, sin abrir un archivo. `bilinker log --first-parent <rama> ^<mi-rama>` lo acota a lo que la otra rama decidió y ésta todavía no tiene, que es la pregunta previa a un `adopt`.

### `bilinker diff` compara `.bilink/` contra el commit de la ref del que salió

Muestra qué hay en el `.bilink/` del árbol de trabajo que la ref todavía no tiene, sobre el índice propio y no sobre el del proyecto. Vacío quiere decir que todo lo aceptado está en la ref, y es la comprobación previa a publicar. Con `--against <ref>` compara contra las aceptaciones de otra parte, por ejemplo la de un proveedor, en vez de contra el commit del que salió.

## Invariantes

1. Los bilinks viven en `refs/bilink/<branch>` y ninguna rama del proyecto los contiene.
2. El árbol de código de todo commit de la ref es idéntico al del commit del proyecto absorbido más recientemente.
3. Ningún commit sobre la ref se escribe sin que el commit del proyecto contra el cual se calculó esté absorbido, y la absorción ocurre en un commit propio, nunca en el mismo.
4. Un commit sobre la ref hace una cosa, y es una de tres: absorción —dos padres, uno del proyecto, diff de `.bilink/` vacío—, decisión —un padre, árbol de código sin cambios—, o sincronización —dos padres, los dos de la ref, árbol de código sin cambios—. Ninguno hace dos.
5. Una aceptación es un commit de tipo decisión por endpoint aceptado, y las de una misma invocación son hijas de la misma absorción.
6. La ref no se rebasea ni se cherry-pickea nunca. Es append-only, todo fetch es fast-forward, y sus únicos escritores son los comandos de bilinker.
7. La ref es protegida: en el servidor, todo push que no sea fast-forward y todo delete se rechazan. Localmente la violación no se impide, se detecta.
8. El árbol del commit de un commit sobre la ref se construye con `read-tree` del absorbido más `update-index` de `.bilink/`. Nada del árbol de trabajo fuera de `.bilink/` entra.
9. La ref es por repo: cubre todas las capas de ese repo y ninguna de un repo anidado.
10. `.bilink/head` dice a qué rama y a qué commit de la ref corresponde el `.bilink/` del árbol, lo escriben tanto la materialización como todo commit sobre la ref, y ningún comando opera sobre un `.bilink/` que no corresponde a la rama actual.
11. La puesta a punto de un clon —exclusión, refspec y materialización— es `init`, es explícita, y sin ella ningún comando corre.
