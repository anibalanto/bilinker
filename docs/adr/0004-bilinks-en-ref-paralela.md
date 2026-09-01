# ADR-0004: bilinker — Los bilinks viven en una ref paralela

**Estado:** Propuesto **Fecha:** 2026-08-28 **Actualizado:** 2026-08-29

**Parte de** [la épica del MVP](../../../../../../.stratum/worklist-accreta/1.epic.md). Usa el vocabulario de [ADR-0003](0003-formato-captures-y-aceptacion.md) —`accepted`, el `commit` derivado, `cache/state`— y es precondición de [ADR-0005](0005-frontera-entre-proyectos.md), porque lo que el consumidor trae del proveedor es exactamente esta ref.

---

## Contexto

**Lo que compra es adopción.** El endpoint `abstract` de [ADR-0005](0005-frontera-entre-proyectos.md) le pide a un proyecto como `hsi` —con mucha gente que nunca oyó hablar de bilinker— que acepte carpetas nuevas en su rama principal. Eso convierte el costo de ser proveedor en una negociación con el equipo entero, y no en una decisión técnica.

De [ADR-0003](0003-formato-captures-y-aceptacion.md) hacen falta dos cosas, y son las que vuelven posible este diseño: que **el estado sea derivado** y viva fuera de git, para que `check` no produzca ningún commit; y que **`accept` falle sobre un archivo sucio**, que es lo que impide que la ref absorba un commit que no contiene lo que se aceptó.

---

## Decisión

### 1. Los bilinks viven en una ref paralela

Ninguna rama del proyecto contiene `.bilink/`. Los bilinks viven en **`refs/bilink/<branch>`**, una ref por rama del proyecto: los de `rc-2.35` están en `refs/bilink/rc-2.35`.

Lo que compra es adopción. Sin esto, [ADR-0005](0005-frontera-entre-proyectos.md) le pide a un proyecto como `hsi` —con mucha gente que nunca oyó hablar de bilinker— que acepte carpetas nuevas en su rama principal. Con esto, `hsi` publica abstracciones y su `main` no cambia un byte. El costo de ser proveedor deja de ser una negociación con el equipo entero.

**Fuera de `refs/heads/`.** La ref no es una rama: `git branch -a` no la lista, la UI de la forja tampoco —los listados de ramas muestran `refs/heads/*`— y `git log --branches` la ignora. Es lo que hacen `git notes` con `refs/notes/*` y Gerrit con `refs/changes/*`. Requiere refspecs explícitos para push y fetch, que los pone bilinker; nadie los tipea. Así los bilinks no existen para quien no los busca, que es más fuerte que tenerlos en una rama aparte.

**Contenido: el árbol del proyecto más `.bilink/`.** No una rama huérfana con sólo los bilinks. Cada commit es entonces un snapshot consistente por construcción, y eso es lo que hace simple el caso remoto: el consumidor trae **una sola ref** y obtiene las declaraciones del proveedor junto con exactamente el código al que apuntan, sin tener que traer dos refs y confiar en que se correspondan. El árbol no se duplica: git comparte los objetos con la rama del proyecto, así que el costo marginal de cada snapshot es la carpeta `.bilink/` y nada más.

#### Invariante de fidelidad del snapshot

Son dos enunciados, los dos verificables sin tree-sitter:

> **1.** El árbol de código de todo commit de `refs/bilink/<branch>` es **idéntico al del commit del proyecto absorbido más recientemente** — el segundo padre del merge más cercano siguiendo primeros padres, o el propio si el commit *es* ese merge.
> **2.** El commit del proyecto contra el cual `accept` o `apply` calcularon su trabajo tiene que estar absorbido antes de commitear.

El primero es una comparación de tree oids: exacta, y barata porque el merge más cercano casi siempre es el commit mismo o el anterior. Es lo que garantiza que el consumidor remoto vea exactamente lo que el proveedor tenía en ese commit — ni una foto vieja ni una incoherente. El segundo garantiza que ese commit absorbido sea **el correcto**. Y la exigencia de [ADR-0003](0003-formato-captures-y-aceptacion.md) —`accept` falla sobre un archivo sucio— es lo que impide que el enunciado 2 se cumpla sobre un commit que no contiene lo que se aceptó.

Con eso, **absorber deja de ser un comportamiento por comando y pasa a ser una precondición de todo commit sobre la ref.** No hay una tabla de quién absorbe y cuándo, ni un `sync` implícito adentro de cada comando: hay una condición que se verifica y, si no se cumple, se cumple absorbiendo **en el mismo commit**. Cuando ya se cumple —el proyecto no se movió desde la última absorción— el acto es un commit común de un solo padre, y el enunciado 1 sigue valiendo porque su árbol de código no cambió.

**Lo que la invariante NO dice**, y no debe decir: que los bilinks del commit estén en `OK`. Una versión más fuerte —*"todo `accepted.link` resuelve, contra el árbol de ese commit, a su `accepted.hash`"*— prohibiría que la ref contenga drift, cuando el drift es el estado normal que la herramienta existe para reportar. Con `bilinker track` queda evidente: una rama nueva hereda bilinks que describen código de otro commit, y casi seguro arranca con drift. Correcto que lo haga.

`check` sigue sin producir ningún commit: por [ADR-0003](0003-formato-captures-y-aceptacion.md) el estado es derivado y vive fuera de git. Atarlo a un commit reintroduciría el defecto (b) empeorado — un commit por corrida en vez de 16 archivos sucios.

#### Cómo se arma el commit de la ref

```
read-tree     <commit del proyecto absorbido>       ← el nuevo, si hay que absorber;
update-index  únicamente .bilink/                     el vigente, si ya está absorbido
```

Nada del árbol de trabajo fuera de `.bilink/` entra jamás al commit de la ref. `cache/`, `index/`, `head` y `.bilink-migrate-*` quedan fuera del índice de bilinker, no sólo del índice del proyecto.

**Verificaciones antes de cada commit:**

1. **Disyunción.** El **árbol** del commit del proyecto que se absorbe no contiene `.bilink/`. Si lo contiene, alguien mergeó la ref al proyecto o commiteó bilinks a mano.
   Se aborta. La reparación —cómo volver a sacar `.bilink/` de la rama del proyecto, y qué hacer con lo que había ahí— queda diferida.
   Es sobre el árbol y no sobre el diff a propósito: el commit que *borra* `.bilink/` tiene un diff que lo toca y un árbol que no, y es exactamente el commit que hay que poder absorber — el `X` del corte es exactamente eso.
2. **Fidelidad.** El árbol de código del commit nuevo es idéntico al del commit del proyecto absorbido vigente. Comparación de tree oids; si falla, abortar.

Las dos corren sobre lo que bilinker está por escribir. Verificar una ref **ajena** —la de un proveedor, construida por otra herramienta, donde el consumidor no tiene otra fuente contra la cual contrastar— es un problema distinto y queda diferido.

Con eso, *"la ref nunca se mergea de vuelta"* deja de ser una regla de buena conducta y pasa a ser una condición **chequeada**. No es exigible como invariante —nadie puede impedir que alguien mergee— pero sí es detectable antes de que contamine nada, que es lo que hace falta para que el resto del diseño se sostenga.

**Evolución: merge, y en una sola dirección.** Cuando la rama del proyecto avanza, la ref de bilinks la absorbe con un merge. Nunca rebase —reescribiría la historia que el `get --diff` de [ADR-0005](0005-frontera-entre-proyectos.md) necesita recorrer hacia atrás— y nunca cherry-pick, que copia los commits en vez de referenciarlos: los del proyecto dejarían de ser ancestros, cada ronda siguiente conflictuaría por falta de base común, y se perdería la correspondencia.

Los merges no conflictúan: la ref de bilinks **no modifica archivos del proyecto** —su único diff es agregar `.bilink/`— y el proyecto no toca `.bilink/`. Los dos lados escriben conjuntos disjuntos, y eso se cumple **desde el corte** porque la ref nace de un commit del proyecto en el que `.bilink/` ya no existe .

#### La ref en operación

**Un merge sólo aparece cuando hay algo nuevo que absorber.** Si el proyecto no se movió desde la última absorción, el `accept` o el `apply` es un commit común de un solo padre: su árbol de código ya es el correcto y no hay nada que traer. El merge no es la forma del acto, es la forma de *ponerse al día*, y cuando hace falta se hace en el mismo commit del acto en vez de en uno aparte.

```
rc-2.35:             X ─────── B ─────── C ─────────────────── D ─────── E
                     │                   ╲                     ╲         ╲    ← 2º padre: lo que se absorbe
refs/bilink/rc-2.35: ●0 ───────────────── ●1 ─────── ●2 ─────── ●3 ────── ●4
                     corte                accept     apply      accept    sync
                                          (merge C)  (1 padre)  (merge D) (merge E)
```

| | Acto | 2º padre | Diff de `.bilink/` contra su 1er padre |
|---|---|---|---|
| `●0` | el corte | — | agrega `.bilink/` entero |
| `●1` | `accept 7f3d8e9a.0` | `C` | `accepted` completo de un endpoint |
| `●2` | `apply -y` (3 renames) | — | tres `link` repuntados; ningún `accepted` tocado → los tres quedan `RELOCATED` |
| `●3` | `accept . --place` | `D` | los tres `accepted.link` → los tres vuelven a `OK` |
| `●4` | `sync` | `E` | **vacío** — el proyecto avanzó y nadie aceptó nada |

`●2` es el caso que hace la diferencia: nadie tocó el proyecto entre `●1` y él, así que no hay merge — y sin embargo su árbol de código sigue siendo el de `C`, que es lo que la invariante de fidelidad pide. `●3` sí absorbe, porque el `accept` se calculó contra `D`.

`●2` y `●3` son además el par que [ADR-0003](0003-formato-captures-y-aceptacion.md) exige: `apply` describe y `accept` bendice, dos commits porque son dos actos, con dos autores posibles. `●4` es el único cuyo diff de `.bilink/` es vacío, y por eso el único que no registra ninguna decisión: alinea la foto y nada más. Deshacer `●1` sería un `●5` que reescribe esos mismos tres campos con los valores leídos de `refs/bilink/rc-2.35~4` — un commit más hacia adelante, nunca un `revert`.

El corte (`●0`) es el único commit de la ref sin ningún commit del proyecto absorbido *por debajo*: nace de `X` como padre único, y ahí la fidelidad se lee contra `X` mismo.

**La correspondencia con el proyecto es el segundo padre**, y por lo tanto un hecho de git y no una convención de nombres: se recorre con `git log --parents`, y `git branch --contains` y `git merge-base` la responden solas. Un commit de un solo padre la hereda del merge más cercano hacia atrás, que es la misma lectura y el mismo recorrido. No hace falta ningún identificador propio — el hash del commit ya lo es, y el estado anterior es `refs/bilink/rc-2.35~1`.

Y se lee bien: **`git log --first-parent refs/bilink/rc-2.35` muestra sólo la evolución de los bilinks**, ocultando la historia del proyecto absorbida.

```
$ git log --first-parent --format='%h %an  %s' refs/bilink/rc-2.35
b1e3f55  Kim     sync: rc-2.35 hasta E
9c1f0ab  Ana     accept --place: 3 endpoints tras el rename de sciplink.rs
4e77d20  Ana     apply: 3 renames — sciplink.rs → scip/link.rs
77a0c94  Luis    accept 7f3d8e9a.0: spec de check ↔ check_structural
0af3c12  Luis    corte: bilinker-003-immutable-captures
```

Ése es el registro de decisiones: quién aceptó qué y cuándo, sin una sola línea del historial del proyecto de por medio.

#### Autoría, atestación y autorización

Tres cosas distintas que conviene no mezclar:

| | Qué es | Con qué se hace | ¿Existe hoy? |
|---|---|---|---|
| **Atribución** | quién *dice* que fue | `author`/`committer` del commit de la ref | no — `accept` no registra a nadie |
| **Atestación** | prueba de que fue | `git commit -S` · `gpg.format=ssh` · `git verify-commit` | disponible, offline, sin infraestructura |
| **Autorización** | si tenía derecho a aceptar | gobernanza | no, y está bien que todavía no |

Para auditar y revertir alcanza con la atribución; ante alguien que no confía, con la firma. El autor de git es auto-declarado: lo que constituye atestación es la firma, no el campo. El `--first-parent` de § "La ref en operación" es ese registro; lo que lo vuelve prueba y no relato es que cada línea sea un commit firmado.

Ésa es la respuesta a por qué la aceptación no necesita un archivo propio para ser revisable: **la superficie de revisión es la de bilinker, no la de la forja.** La forja no muestra la ref, pero `status`, `diff` y `log` sobre el índice y la ref propios sí, y el artefacto firmable es el commit.

**Granularidad: un commit por invocación de `accept`, no por aceptación.** La granularidad sigue al **acto**, no al objeto. `accept <uuid>.0` da un commit; `accept .` da un commit, con el mensaje enumerando los endpoints — porque es una persona mirando y decidiendo **una vez**, y partirlo en N commits firmados no agrega verdad, sugiere N decisiones donde hubo una. Encaja con lo que ADR-0007 ya exige: después de reescribir las specs los NO-OK se cierran endpoint por endpoint, y en ese flujo los commits salen individuales solos. `accept .` queda para lo legítimamente masivo — endpoints `PENDING` de una cadena nueva, propagación `CHAIN_DIRTY` — donde el lote sí es la decisión.

**Deshacer una aceptación** no necesita `git revert`: es **reescribir su `accepted` con los valores anteriores**, un commit nuevo, y funciona igual venga de un lote o de un commit individual. Los valores viejos se leen de `refs/bilink/<branch>~n`. Absorber sigue siendo una sola vez antes del lote.

**Bendecir lo mismo no diluye la responsabilidad.** Dos bilinks pueden tener un `accepted` idéntico, pero los actos que los escribieron son dos commits distintos, cada uno con su autor y su firma. La responsabilidad vive en el commit que escribió el valor, no en el valor.

**No hay riesgo de colisión de hashes por volumen.** SHA-1 son 160 bits y el límite de cumpleaños está en ~2⁸⁰ objetos; el kernel de Linux tiene ~2²³. Desde git 2.13 el algoritmo es SHA-1DC, que detecta las construcciones tipo SHAttered y aborta. Y los hashes propios de bilinker —ids de capture, `accepted.hash`, `accepted.hash_ast`— ya son SHA-256, así que el direccionamiento por contenido de este ADR es más fuerte que la capa de objetos de git. Lo único que crece de forma relevante es la longitud del walk hacia atrás de `get --diff`, y como la ref es lineal y append-only eso se resuelve con bisección si alguna vez llega a importar.

#### Los `.bilink/` están en el árbol de trabajo

No en un worktree aparte. Quien usa bilinker o lattice quiere esos archivos a mano, así que se materializan junto al código, y el proyecto los ignora. La exclusión va en **`.git/info/exclude`, no en `.gitignore`**: `.gitignore` está versionado, y agregarlo modificaría la rama del proyecto — justo lo que este diseño evita. `info/exclude` es local, no se commitea y no aparece en ningún MR.

Con eso `check` corre en caliente: bilinks y código vivo en el mismo directorio, con los cambios sin commitear a la vista. Es lo que `check.md:109` ya exige al comparar contra el árbol de trabajo y no contra HEAD, *"porque los cambios sin commitear quedan invisibles — que es el caso más común mientras alguien trabaja"*. Si se comparara contra la copia de código de la ref, quien rompe un vínculo no se enteraría hasta que alguien sincronice, y la detección de drift llegaría siempre tarde.

**Un índice propio.** Ignorados a secas, los cambios que escribe `accept` no aparecerían en ningún `git status`: para el índice del proyecto son archivos ignorados, y la ref donde cuentan no está checkouteada. Bilinker usa su propio `GIT_INDEX_FILE` sobre el mismo árbol de trabajo, contra `refs/bilink/<branch>`. El mismo `.bilink/` queda ignorado por el índice del proyecto y trackeado por el de bilinker, que así recupera `status` y `diff` reales sin ensuciar los del proyecto. Es el patrón conocido de los dotfiles en repo bare.

**La asimetría local/remoto, dicha explícitamente.** Localmente el código sale del árbol de trabajo, así que la foto de la ref puede estar atrasada sin afectar un `check`. Remotamente el consumidor sólo tiene la ref, así que esa foto **es** el código con el que verifica. Por eso la absorción no es un requisito para observar, sino para que el snapshot sea cierto para quien no tiene otra cosa.

`sync` cubre el caso en que el proyecto avanzó y nadie aceptó nada. Alinea la ref con el proyecto y **no verifica nada** — de ahí el nombre; `update` sugeriría que recalcula estados, que es lo que no hace.

#### `bilinker init` — la puesta a punto del clon

**Es lo primero que corre cualquiera que vaya a usar bilinker.** Todo lo anterior necesita tres cosas puestas en el clon, y ninguna viaja con él: la exclusión en `.git/info/exclude`, el refspec en `.git/config`, y el `.bilink/` materializado en el árbol. Son **por clon**, no por rama ni por commit, porque las tres viven en `.git/` o fuera de git.

```
bilinker init

1. .git/info/exclude  ←  .bilink/
2. .git/config        ←  refspec de refs/bilink/*   → desde acá, git fetch las trae
3. fetch de refs/bilink/*  +  materializar el .bilink/ de la rama actual  +  escribir head
```

Es idempotente y es **por repo**, no por capa: un solo patrón `.bilink/` en el exclude cubre las capas de ese repo, estén donde estén.

**El paso 3 no pisa nada.** Si hay un `.bilink/` en el árbol y no hay `head`, `init` no puede saber de dónde salió, así que lo deja intacto y se limita a los pasos 1 y 2 — y lo dice. Es lo que hace que el paso 5 del corte pueda ser un `init` a secas: ahí el `.bilink/` recién renombrado todavía no está en la ref, y materializar lo borraría.

**Sin `init`, los comandos fallan; no se auto-configuran.** Y la línea de por qué es la misma que separa lo automático de lo que se pide: **bilinker arregla solo lo que es suyo, y pide lo que es del repo del usuario.** Materializar `.bilink/` es automático porque `.bilink/` le pertenece; escribir en `.git/config` y `.git/info/exclude` es tocar la configuración de otro, y eso merece un acto explícito por más inofensivo que sea. La alternativa —que el primer `check` configure el repo de callado— convierte un comando de lectura en uno que modifica el entorno.

Y esto absorbe el paso 5 del corte: la migración deja de tener setup propio y corre el mismo `init` que corre cualquier clon. Un camino menos que mantener, y uno que se ejercita todos los días en vez de una sola vez.

**Y con el refspec puesto, `git fetch` trae las refs de bilinks junto con las ramas.** Sin él, una rama al día puede convivir con bilinks viejos, que es la clase de desajuste silencioso que este diseño existe para eliminar. Es una línea en `.git/config`, local y no commiteada, de la misma clase que el `.git/info/exclude` que va al lado, y no compromete la invisibilidad: `refs/bilink/*` sigue sin aparecer en `git branch -a` ni en la forja, porque no está bajo `refs/heads/` ni bajo `refs/remotes/`. Es lo que § "Fuera de `refs/heads/`" ya anticipaba al decir que los refspecs *los pone bilinker*.

#### Cambiar de rama

Materializar `.bilink/` en el árbol y excluirlo del índice del proyecto tiene un filo: **`git checkout` no lo toca**. Para el índice del proyecto son archivos ignorados, así que cambiar de rama mueve el código y deja los bilinks donde estaban. Quedás con el código de `B` y los bilinks de `A`, y nada te avisa.

**El árbol dice de dónde salió: `.bilink/head`.** Un archivo con dos valores —la rama y el commit de `refs/bilink/<branch>` que le corresponde— que escribe tanto la materialización como **todo commit sobre la ref**: si `accept` avanza la ref, el árbol pasa a corresponder al commit nuevo y `head` tiene que decirlo, o la guarda de abajo se dispararía después de cada aceptación. No se commitea: se suma a `cache/` y `index/` en la lista de lo que queda fuera del índice de bilinker. No es configuración, es estado del árbol de trabajo, y es exactamente lo que `HEAD` es para git.

**Sin punto**, aunque `.bilink/` tenga adentro un `.{alias}.toml`. La regla que gobierna ese punto es la de Stratum, donde un dotfile describe a su hermano sin punto —`.stratum/.impl.toml` describe a `impl/`, y acá `.hsi.toml` describe al clon en `hsi/`—; `head` no describe a nadie. Y `.bilink/` ya es un directorio oculto: adentro, el punto no esconde nada y sólo agrega ruido, que es por lo que `cache/state` e `index/index` tampoco lo llevan, y por lo que git escribe `HEAD` y no `.HEAD` dentro de `.git/`.

Con eso la corrección es **automática y sin ceremonia**: cualquier comando compara `head` contra la rama del proyecto, y si no coinciden materializa el `.bilink/` de la ref correcta y sigue. No hay comando de más que tipear, ni pregunta que contestar — no hay nada que decidir.

**Una sola guarda, la de git.** Si el `.bilink/` del árbol difiere del commit que `head` nombra, hay trabajo que no está en ninguna parte —`.bilink/` está fuera del git del proyecto— y materializar lo destruiría. Ahí se para y se avisa, igual que `git checkout` se niega a pisar cambios. En el flujo diseñado esa ventana no existe: `accept` y `apply` commitean sobre la ref como parte del acto, así que el árbol queda limpio contra el índice propio. Queda abierta sólo para un `apply` que crashea a mitad o una edición a mano, y las dos merecen un humano.

**Por qué un archivo y no inferirlo de `cache/state`.** Porque la cache es un derivado y estar fría es un estado normal —clon fresco, otra máquina, `rm -rf .bilink/cache/`—, o sea que no puede responder justo cuando más falta hace. Y peor: haría que un derivado autorice sobrescribir la fuente, que es la misma asimetría fatal por la que el `prune` de la Decisión 1 no puede usar el índice como autoridad. La procedencia del `.bilink/` es un hecho sobre el árbol, no algo que se recalcula.

Son **dos marcadores para dos preguntas distintas**, y los dos hacen falta: `head` responde *"¿son éstos los bilinks que corresponden?"* y protege la fuente; el commit que anota `cache/state` responde *"¿estos estados se calcularon sobre estos bilinks?"* y protege el derivado.

`bilinker track` queda entonces con un solo trabajo —crear la ref de una rama que no la tiene— más servir de escape explícito para re-materializar a mano.

**`head` no se mete con el rebase, y está bien que no.** Un `git rebase main` estando en `feature/x` te deja en `feature/x`, y no toca `refs/bilink/feature/x`: `head` sigue diciendo `(feature/x, ●a)`, el `.bilink/` del árbol sigue siendo el de `●a`, y todo coincide. No hay materialización que disparar, porque los bilinks no se movieron — se movió el código debajo. Eso es otro problema, y lo atiende § "Cuando la rama se rebasea".

Vale enunciarlo como reparto, porque son tres desajustes distintos que se detectan y se arreglan distinto:

| Qué se movió | Cómo se detecta | Qué lo arregla |
|---|---|---|
| la rama (`checkout`) | `head` ≠ rama actual | materializar — automático, sin comando |
| el código de la rama (commit, rebase, merge) | el commit absorbido ≠ tip de la rama | `sync`, o el `accept`/`apply` siguiente, que absorbe igual |
| las decisiones del vecino (rebase sobre otra rama) | `status` lo avisa: la ref del vecino avanzó | `bilinker adopt <rama>` — con `--dry-run` para verlo antes |

Sólo el primero puede resolverse solo, y por eso es el único que se resuelve solo: los otros dos escriben un commit sobre la ref, y ninguno de los dos es una decisión que la herramienta pueda tomar por su cuenta.

**En `HEAD` desacoplado no se materializa nada.** A mitad de un rebase con conflictos no hay rama actual contra la cual comparar, y adivinar una sería peor que no hacer nada. Ahí los comandos de lectura corren contra lo que `head` dice, avisando; los que commitean sobre la ref se niegan hasta volver a una rama.

#### Auditoría: contra la ref, no contra la rama

La rama del proyecto no está protegida: se rebasea, se force-pushea, se cambia. Pero la ref **absorbe los commits del proyecto como segundos padres**, así que todo commit alguna vez absorbido queda alcanzable desde la ref para siempre. La arqueología corre ahí:

```
git merge-base --is-ancestor <commit> refs/bilink/<branch>   ← precondición
git log <commit>..refs/bilink/<branch> -- <file>              ← la ventana
```

Si aun así no es ancestro (commit nunca absorbido, clon superficial del proveedor), degradar a *"fuente desconocida"* en vez de devolver un rango inflado.

#### `bilinker track`

Sin él, empezar a seguir una rama nueva deja todos los endpoints en `PENDING`. Compone directo con `bilinker/architecture.md` § "Implementaciones alternativas por branch".

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

`feature/x` sale de `D`, y la ref de `main` ya absorbió `E`:

```
main:                  X ─── B ─── C ─── D ─── E
                       │           ╲     ╲     ╲
refs/bilink/main:      ●0 ───────── ●1 ── ●2 ── ●3

feature/x:                               D ─── F ─── G
                                                     ╲
refs/bilink/feature/x:                    ●2 ──────── ●a
```

`●a` es un commit de la misma forma que cualquier otro de una ref, sólo que sus dos padres vienen de lugares distintos: el **primero** es `●2`, de donde hereda los bilinks; el **segundo** es `G`, de donde saca el código. Su árbol de código es el de `G` y su `.bilink/` es el de `●2`, así que su diff contra el primer padre es vacío — `track` no decide nada, igual que `sync`.

**`●2` y no `●3`.** Los candidatos son los commits de la cadena de primeros padres de `refs/bilink/main`, y se los mira por su segundo padre: el de `●1` es `C`, el de `●2` es `D`, el de `●3` es `E`. `C` y `D` siguen siendo ancestros de `feature/x`; `E` no, porque la rama se bifurcó antes. `D` es el más nuevo de los que califican, y por eso gana `●2`. Heredar de `●3` traería bilinks que describen código que `feature/x` no tiene.

**La búsqueda va de la ref hacia el proyecto, nunca al revés.** Ningún commit del proyecto tiene un merge a `refs/bilink/*` — la relación es exactamente la inversa. Buscar en los ancestros de la rama nueva un merge hacia la ref sólo puede encontrar el bug que la verificación de disyunción detecta.

El test `--is-ancestor` es lo que traduce *"la última versión de los bilinks accesible"* a algo exacto. En el caso común —la rama sale del tip de una rama trackeada y la ref está al día— el `P` más nuevo es el commit base de la rama nueva y el paso 2 termina en la primera iteración. Los tres casos donde el filtro deja de ser cosmético:

| Situación | Qué pasa sin el test | Qué hace `track` |
|---|---|---|
| La ref va **adelante** del punto de fork (rama sale de `D`, la ref absorbió `E`) | hereda bilinks que apuntan a código que la rama no tiene → `UNRESOLVED` y `ALTERED` falsos desde el minuto cero | toma el `M` cuyo `P` es `D` |
| **Ningún `P` califica** (la rama sale de antes del corte, o de una línea nunca trackeada) | elegiría algo parecido | lo dice y crea la ref desde cero |
| **Califican varias refs** (el fork es ancestro de `main` y de `rc-2.35`) | adivina | exige `--from` |

#### Cuando la rama se rebasea

Lo que este ADR prohíbe es rebasear **la ref**, no la rama del proyecto. `feature/x` se rebasea cuando su dueño quiera; la ref contesta con merges. Pero en un rebase pasan **dos cosas distintas**, y conviene que sean dos commits:

```
refs/bilink/main:           ●2 ── ●3 ─────────────────────╮
                                                          │    2º padre de ●c: trae ●2..●3,
feature/x (rebaseada):            E ─── F' ─── G'         │    las decisiones de main
                                                 ╲        │
refs/bilink/feature/x:      ●a ────────────────── ●b ──── ●c
```

**`●b` — absorber `G'`.** No tiene nada de especial: es la absorción de siempre, y no hay que pedirla. `adopt` escribe un commit sobre la ref, así que la precondición de fidelidad lo obliga a absorber `G'` antes; `sync` hace lo mismo para quien sólo quiere ponerse al día sin traer nada del vecino. Es obligatorio en cualquier caso, porque si no la ref sigue afirmando que su código es `G`, un commit que el rebase abandonó.

**`●c` — traer `●2..●3`.** Rebasear sobre `main` metió el código de `E` en la rama; si `main` aceptó algo sobre ese código en `●3`, los bilinks heredados en `●2` no lo tienen y van a reportar drift que `main` ya resolvió. Hace falta un comando nuevo, **`bilinker adopt <rama>`**, y sólo cuando la ref del vecino avanzó.

**No se llama `merge` a propósito.** En este diseño *merge* ya nombra una cosa muy precisa y estructural: un commit de la ref que absorbe un commit del proyecto como segundo padre. Es la palabra con la que están escritas la invariante de fidelidad, la evolución de la ref y el diagrama de § "La ref en operación". Usarla también para el comando que reconcilia dos refs de bilinks la sobrecargaría justo donde el documento necesita precisión — el mismo motivo por el que `sync` no se llama `update` y `REMOTE_UNREACHABLE` no se llama `REPO_`. `adopt` dice lo que pasa y es asimétrico, que es la verdad: las decisiones de `main` entran acá, y ninguna de las mías va para allá. Que su implementación produzca un commit de merge se sigue pudiendo decir sin ambigüedad, porque el verbo y el mecanismo dejaron de compartir nombre.

Se nombra la **rama del proyecto**, no su ref de bilinks: `bilinker adopt origin/main`, y la traducción a `refs/bilink/main` la hace la herramienta. Es la misma regla que ADR-0004 ya fija para el `.toml` del endpoint repo —*"el `.toml` declara la rama del proyecto, y la herramienta traduce a su ref"*—, y por la misma razón: una sola fuente de verdad, y nadie tipeando namespaces de refs.

**Ver qué decidió el vecino antes de traerlo** no necesita un verbo propio. Son dos preguntas y las dos ya tienen respuesta en el vocabulario del ADR:

- *¿Qué actos hubo del otro lado?* — `bilinker log --first-parent <bilink-ref> ^<mi-ref>`, que es el registro de decisiones de § "La ref en operación" acotado al rango que falta. Sale de la superficie de revisión que ADR-0006 ya paga.
- *¿Qué le harían a mis bilinks?* — **`bilinker adopt <rama> --dry-run`**, con el mismo contrato que `migration.md` ya define para `--dry-run`: calcula y reporta exactamente lo mismo, sin escribir un solo archivo.

```
$ bilinker adopt origin/main --dry-run
base ●2 · 4 aceptaciones de main en ●2..●3

entra limpio     7f3d8e9a.0   contenido    Luis
                 a3f9c821.1   ubicación    Ana
ya coincidía     c1a2b3c4.0   contenido    — mismo valor de los dos lados
conflicto        d5e6f7a8.0   contenido    main 838ea0a4…  ·  acá 9211a4f3…
```

Las cuatro filas son las únicas posibles, y salen del formato sin nada agregado: `accepted` son campos con nombre, por endpoint, así que un merge a tres puntas los compara de a uno. **Que la fila "ya coincidía" exista es la convergencia** que la Decisión 1 hace posible — dos personas que aceptan el mismo contenido en HEADs distintos escriben los mismos valores — y por eso el caso común de un rebase no conflictúa nada.

Y la base de ese merge a tres puntas es `●2`, sin que nadie la calcule: es la base de merge real entre `●a` y `●3`, porque `track` puso `●2` como **primer padre** de `●a` en vez de copiar archivos. Ésa es la razón de fondo de la forma que la sección anterior le dio a `track`, y recién acá se cobra.

### 2. El corte, que son dos

Llevar los bilinks a la ref **es un paso del proceso de migración**, no algo aparte. Y eso obliga a una distinción que `concepts/migration.md` todavía no hace:

> Una **migración** es lo que `migration.md` ya define: transformación sintáctica, idempotente, con `--dry-run`. El **proceso de migración** es la transición completa, y tiene pasos que no son migraciones.

Los cortes son de esa segunda clase. Entran en la misma secuencia numerada y el mismo ledger; los corre otro comando, así que `migration.md` inv. 5 —una migración no consulta git— e inv. 6 —el runner nunca commitea— se salvan enteras.

```
bilinker-002-file-partition        transformación
bilinker-003-immutable-captures    transformación
bilinker-004-format-cutover        corte de formato
bilinker-005-ref-cutover           corte a la ref
```

**Y son dos cortes, no uno.** No para poder frenar en el medio —quien quiera el formato nuevo se lleva la ref, porque el runner aplica todo lo no aplicado— sino para **depurar un cambio por vez**. Con un solo corte, el primer problema que aparezca se mira con el formato cambiado **y** los bilinks fuera de la vista de git al mismo tiempo: sin `git status`, sin `git diff`, sin `git branch -a`, porque la ref usa índice y refspecs propios. Es exactamente el escenario que el § "El problema real es de bootstrap" de [ADR-0003](0003-formato-captures-y-aceptacion.md) advierte — si la herramienta se rompe, no queda con qué diagnosticarla.

#### `004` — el corte de formato

```
1. Generar (o regenerar) .bilink-migrate-003…/ en todas las capas.   [sin git]
2. Reemplazar .bilink/ por esa carpeta en el árbol.
3. UN commit en la rama del proyecto con el .bilink/ nuevo.
4. Ledger: 002, 003, 004.
```

Acá el formato nuevo queda vivo **en las ramas**, con git normal. Es donde corren los tests, el `check` sobre los 63 de la raíz y sus 63 contrapartes, y el ciclo de aceptación completo. Nada de la ref todavía.

Y donde corre la verificación de que la migración no perdió nada — que la hace **la migración**, único componente que depende de los dos crates de formato. `check --against` no puede: compara dos carpetas de la misma versión, no cruza formatos.

#### `005` — el corte a la ref

Sólo cuando el `004` está validado entero.

```
1. UN commit que saca .bilink/ del índice de la rama.   → X   (pushear antes de seguir)
2. git update-ref refs/bilink/<branch> X
3. bilinker init  (exclude + refspec)
4. Commit sobre refs/bilink/<branch>: agrega .bilink/.   → ●0, padre X
5. Ledger: 005.
```

El paso 1 es `git rm --cached -r .bilink/` más commit: **los archivos se quedan en disco**, así que no hay que restaurarlos para commitearlos a la ref.

> **Enmienda — los pasos 2 y 4 son un solo `bilinker track`.** Al implementar la Decisión 1 quedó claro que el corte no necesita un camino propio: es exactamente el caso *"ningún `P` califica"* de `track`, que crea la ref con el `.bilink/` del árbol y el tip de la rama como padre único. Y el `update-ref` a `X` no sólo sobra, estorba: dejaría `refs/bilink/<branch>` apuntando a un commit del proyecto, y `track` tendría que tratar un commit ajeno como propio para poder seguir. La secuencia queda en tres pasos —`git rm --cached` + commit, `bilinker init`, `bilinker track <branch>`— más el ledger. Ver [`commands/track.md`](../../../../commands/track.md) § "Sin candidato, la ref nace desde cero".
>
> `sync` **no** puede escribir el corte, y es correcto que no pueda: sin nada que absorber no escribe ningún commit, porque un merge con el mismo segundo padre y el mismo `.bilink/` no dice nada nuevo. En el corte no hay nada que absorber y sí hay algo que escribir.

#### Orden y regla general

**`refs/bilink/<branch>` se crea desde un commit del proyecto en el que `.bilink/` ya no está en el árbol.** Así la base de merge de todo `sync` futuro es `X` o un descendiente, el proyecto no vuelve a tocar `.bilink/` nunca, y el argumento de conjuntos disjuntos se cumple desde `X`. Sin ese orden, el primer merge después del corte produce un modify/delete masivo — o peor, borra en silencio los bilinks que no cambiaron.

*"Revertir es no cortar"* vale hasta el paso 1 del `004`. De ahí en adelante revertir es un `git revert` más borrar la ref — barato igual, pero ya no es no-hacer-nada.

#### Las migraciones no se borran nunca

`migration.md` define ids ordenados, ledger e idempotencia, pero no dice que el conjunto sea de **sólo-agregar**. Y hace falta: es lo único que permite que alguien parado en una versión vieja llegue a la actual corriendo la cadena entera. Sacar una migración porque "ya nadie está en ese formato" deja varado a quien sí lo está.

El corolario, que conviene tener a la vista: **la historia de formatos sí se acumula** — en el build, no en el camino de lectura. Cada migración depende de los dos formatos que puentea, así que los parsers viejos quedan en el repo para siempre. Es el costo del esquema, y es distinto de lo que `migration.md` descarta, que es que **cada lectura** cargue con toda la historia.


---

## Consecuencias

**Invariantes nuevas.** Los bilinks viven en `refs/bilink/<branch>` y ninguna rama del proyecto los contiene · el árbol de código de todo commit de esa ref es idéntico al del commit del proyecto absorbido más recientemente · ningún commit sobre la ref se escribe sin que el commit del proyecto contra el cual se calculó esté absorbido · la puesta a punto de un clon —exclusión, refspec y materialización— es `bilinker init`, es explícita, y sin ella ningún comando corre · `.bilink/head` dice a qué rama y a qué commit de la ref corresponde el `.bilink/` del árbol, lo escriben tanto la materialización como todo commit sobre la ref, y ningún comando opera sobre un `.bilink/` que no corresponde a la rama actual.

**Lo que cuesta.** Bilinker pasa a manejar su propio índice git y sus propios refspecs, y gana comandos: `init`, `sync`, `track` y `adopt`. Las escrituras siguen siendo I/O de archivos normal sobre el árbol de trabajo —los `.bilink/` están ahí, sólo que excluidos del índice del proyecto— así que no hace falta plumbing ni un worktree aparte. Lo que sí cambia de naturaleza es que la herramienta ahora administra una ref: crearla, absorber, empujar, traer y verificar; y que su superficie de revisión —`status`, `diff` y `log` sobre el índice y la ref propios— pasa a ser parte del producto, porque la forja no la muestra. El ledger `.accreta/migrations` la acompaña, porque es metadata de bilinker sobre archivos de bilinker.

**Efecto por comando.** `check` sigue sin commitear nada · `accept` y `apply` exigen que el commit contra el que se calcularon esté absorbido, absorbiéndolo en el mismo commit si no lo está · **`init`, `sync`, `track` y `adopt` —con `--dry-run`— son nuevos**, más `log` y `diff` sobre la ref propia; y todo comando materializa solo el `.bilink/` de la rama actual cuando `.bilink/head` no coincide.

**Lattice gana una segunda dependencia.** Además del `check` que [ADR-0003](0003-formato-captures-y-aceptacion.md) le agrega, antes de eso hacen falta el fetch de la ref y la materialización de `.bilink/` — que es automática vía `.bilink/head` una vez que la ref está, pero que en un clon fresco no ocurrió nunca. Son dos dependencias nuevas de la misma naturaleza, y las dos hay que escribirlas.

**Hay que reconciliar con "Implementaciones alternativas por branch".** `bilinker/architecture.md:90` ya usa el pareo de ramas para otro eje: `specs/feature/X` ↔ `impl/feature/X` es variación entre alternativas, y este ADR es separación entre bilinks y contenido. Componen —`refs/bilink/feature/X`, que es lo que `bilinker track` consume— pero el documento tiene que decirlo, o los dos usos del mismo mecanismo se leen como uno solo.

**Trabajo que dispara en las specs.** Son nuevos y no existen `commands/init.md`, `commands/sync.md`, `commands/track.md` ni `commands/adopt.md`. La reconciliación con `architecture.md` § "Implementaciones alternativas por branch".

---

## Alternativas descartadas

- **Bilinks en las ramas del proyecto**, como hoy. Obliga a cada proveedor a aceptar carpetas `.bilink/` en su rama principal, que para un proyecto con muchos participantes ajenos a bilinker es una negociación y no una decisión técnica.
- **Rebasear la ref de bilinks sobre la del proyecto.** Reescribe historia de forma permanente: los commits que el consumidor necesita alcanzar para responder "¿qué cambió desde que acepté?" dejan de existir, y cada fetch pasa a ser non-fast-forward.
- **Cherry-pick de los commits del proyecto.** Los copia en vez de referenciarlos: dejan de ser ancestros, cada ronda conflictúa por falta de base común, y se pierde la correspondencia con el proyecto.
- **Rama huérfana con sólo `.bilink/`.** Evita el merge, pero obliga al consumidor a traer dos refs —los bilinks de una y el código de otra— y a confiar en que se correspondan. El snapshot completo es coherente por construcción y no cuesta almacenamiento, porque git comparte los objetos.
- **Una rama por estado**, `bilink/<branch>/<id>` con `<id>` incremental o el hash referenciado. Reinventa el historial de commits con nombres de rama: agrega un problema de búsqueda del estado previo que `~1` ya resolvía, y llena el namespace de refs.
- **`refs/heads/bilink/*`** en vez de un namespace propio. Las ramas aparecerían en `git branch -a` y en el listado de la forja, que es justo lo que se quería evitar.
- **Excluir con `.gitignore`** en vez de `.git/info/exclude`. Está versionado, así que modificaría la rama del proyecto — una línea, pero deja de ser cierto que el proyecto no cambia.
- **Leer los bilinks desde un worktree lincado**, sin materializarlos en el árbol. Obliga a traducir paths entre dos árboles y a que `check` elija entre código vivo y código de la ref; y las herramientas que ya miran `.bilink/`, como la extensión de VS Code, no lo encontrarían donde esperan.
- **Absorber sólo en `sync`.** Deja la ref con bilinks nuevos sobre código viejo entre sincronizaciones, violando la invariante de fidelidad: el consumidor remoto resuelve contra un árbol que no es el que el proveedor tenía cuando aceptó.
- **Exigir que todo commit de la ref tenga sus bilinks en `OK`.** Prohibiría el drift, que es el estado normal que la herramienta existe para reportar, y haría imposible `bilinker track`.
- **Un commit por aceptación** en vez de por invocación de `accept`. La granularidad sigue al acto; partir un `accept .` en N commits firmados sugiere N decisiones donde hubo una.
- **Un hook `post-checkout`** para materializar `.bilink/` al cambiar de rama. Es configuración local que hay que instalar en cada clon, y el clon donde no esté instalado es exactamente el que da la respuesta equivocada en silencio. Con `.bilink/head` la corrección ocurre igual en el primer comando que se corra, sin instalar nada.
- **Un `bilinker switch` que envuelva `git checkout`.** Obliga a dejar de usar git para una operación de git, y falla apenas alguien cambia de rama desde el IDE, con `git switch` o en otro worktree. Detectar después del hecho funciona sin importar cómo se cambió la rama.
- **Inferir de `cache/state` de qué rama es el `.bilink/` del árbol**, en vez de un `head` propio. La cache es un derivado y estar fría es normal, así que no responde justo cuando más falta hace; y haría que un derivado autorice sobrescribir la fuente, la misma asimetría por la que el `prune` no puede usar el índice como autoridad.
- **Un solo commit de tres padres** `(●a, G', ●3)` para reconciliar tras un rebase. `--first-parent` mostraría una línea para dos cosas distintas, y la invariante de fidelidad necesitaría una regla acompañante para el tercer padre. Dos commits dejan las dos reglas intactas y no cuestan nada.
- **`git merge` a secas entre dos refs de bilinks.** Como cada una lleva el árbol del proyecto adentro, fusionaría dos árboles de código enteros. El árbol del commit se construye —`read-tree` del absorbido más `update-index` de `.bilink/`—, así que la fusión de contenido queda confinada a `.bilink/`.
- **Auditar contra la rama del proyecto.** No está protegida: se rebasea y se force-pushea, así que un commit registrado puede dejar de ser alcanzable. La ref sí conserva para siempre todo commit alguna vez absorbido.
- **Buscar el ancestro de `track` desde la rama hacia la ref.** Ningún commit del proyecto tiene un merge a `refs/bilink/*` — la relación es exactamente la inversa. Esa búsqueda sólo puede encontrar el bug que la verificación de disyunción detecta.

---

---

## Diferido a una etapa siguiente

Este ADR decide **dónde viven los bilinks**. De ahí cuelga una superficie de operación diaria que no hace falta resolver para tomar esa decisión, y que se resuelve mejor con la ref ya andando. Va a `subsystems/bilinker/proposals/` por la misma regla que ADR-0007 aplica a `kind`, `name.N` y el endpoint de tipo bilink: **lo que está especificado y no implementado no se borra, se mueve** — con su motivación, no sólo con su definición.

Cada punto de abajo tiene un problema real detrás, verificado contra el diseño. Están diferidos, no descartados, y ninguno contradice lo decidido acá.

- **Verificar una ref ajena, y reparar la disyunción rota.** Las dos verificaciones pre-commit protegen lo que bilinker escribe, pero una ref traída de un proveedor la construyó otra herramienta y el consumidor no tiene con qué contrastarla: si su árbol de código no es el que su segundo padre dice, calcula drift contra un árbol fabricado sin forma de enterarse. Un `verify-ref` recorre la ref chequeando los dos enunciados de la invariante —comparaciones de tree oids, sin tree-sitter ni rehashear nada— y **no** debe chequear que los bilinks estén en `OK`, porque una ref con drift es normal. Y cuando la disyunción se rompe hace falta poder repararla, que se cruza con el punto siguiente.
- **Bilinks sin procedencia.** Un `.bilink/` en el árbol que ninguna ref avala, o commiteado en una rama del proyecto, son el mismo problema: decisiones sin un commit firmado detrás. La salida no es descartarlas —sacarlo con un commit sirve para el caso en que llegaron de un merge de la ref al proyecto, pero destruye trabajo si alguien commiteó bilinks a mano— sino adoptarlas, con el commit de adopción atribuyéndolas a quien adopta, que es lo único cierto que se puede decir de ellas. Sin base de merge, toda diferencia es conflicto: ése es el costo de haber perdido la procedencia.

