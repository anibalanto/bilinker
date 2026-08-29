# ADR-0005: bilinker — La frontera entre proyectos

**Estado:** Propuesto **Fecha:** 2026-08-28 **Actualizado:** 2026-08-29

**Parte de** [la épica del MVP](../../../../../../.stratum/worklist/1.epic.md), que lleva el caso `retinar` / `hsi` / `filasvirtuales` y la motivación de entrega. Depende de [ADR-0003](0003-formato-captures-y-aceptacion.md) —copia el `accepted` del vecino— y de [ADR-0004](0004-bilinks-en-ref-paralela.md) —lo que el consumidor trae del proveedor es esa ref—. Es el último de los tres por dependencia, no por bloqueo: **su desarrollo es enteramente propio y se verifica entre dos repos locales.** Lo que depende de gente fuera del proyecto es la adopción — que un proveedor real publique una abstracción— y eso vive en la épica, no acá.

---

## Contexto

Un bilink hoy conecta dos fragmentos *dentro de un proyecto*, aunque crucen repos. Hace falta que conecte fragmentos de **proyectos distintos**, con una restricción fuerte: ningún extremo debe conocer al otro — ni su hash de contenido, ni su ubicación. Y del lado del proveedor van a converger muchos consumidores sobre el mismo fragmento, sin que el proveedor se entere de ninguno.

El caso que lo fuerza está en la épica. Acá van los bloqueos técnicos que impiden resolverlo con lo que hay.

### Bloqueos del modelo actual

1. **El rendezvous es por UUID compartido** y la cadena se localiza por nombre de archivo. Con C consumidores y E fragmentos el proveedor acumula O(C×E) archivos, y sumar o quitar un consumidor exige un commit en su repo.
2. **Cada extremo guarda el hash de contenido del otro.** Es el mecanismo de propagación: `hash.N` de un endpoint layer es copia del `hash.N` estructural del vecino.
3. **El fan-out por capture no sirve en la frontera.** Resuelve la multiplicidad de relaciones sobre un fragmento local; cada una sigue siendo otra cadena con su propio nodo del lado del proveedor.
4. **Ningún path Stratum llega de un proyecto a otro.** `*` es "el ancestro más lejano con `.git`"; repos hermanos sin raíz común no se alcanzan. No hay identidad de capa más allá de un path del filesystem.

Los bloqueos 1, 2 y 4 son consecuencias del mismo hecho: el vínculo entre capas es un puntero mutuo por ubicación. El 3 es consecuencia de la aridad fija más la topología lineal.

---

## Decisión

### 1. El endpoint `abstract`

Una punta que no es responsabilidad de quien la declara:

```yaml
# hsi/.bilink/<uuid>.yaml — bilink abstracto
endpoint:
  0:
    link: capture 8a3f0d21-4e5b-4c3d-9f2a-1b2c3d4e5f6a
    accepted:
      link: capture 8a3f0d21-4e5b-4c3d-9f2a-1b2c3d4e5f6a
      hash: c4e1770b93a5f8d2e6b0c4a8f2d6e0b4c8a2f6d0e4b8c2a6f0d4e8b2c6a0f4d8
  1:
    link: abstract        # lo aporta quien lo consuma
```

Un valor, no un campo ausente: la aridad fija en dos es la invariante más deliberadamente defendida de la spec y no vale la pena romperla por eso. Hay precedente en `TODO`, que ya es "una intención declarada — no es un error"; acá la intención es permanente y abierta a cualquiera.

El proveedor necesita un bilink y no le alcanzan los captures: un capture tiene estado de resolución pero no de aceptación, y *"lo que publiqué sigue coincidiendo con lo que aprobé"* es una pregunta real y puramente local que sólo un bilink sostiene. Además le da una identidad durable que sobrevive a sus propios cambios, y que es lo que los consumidores referencian.

Un endpoint `abstract` no tiene bloque `accepted` en absoluto: no hay nada que bendecir del lado abierto, y con el bloque eso es una ausencia, no una lista de campos que enumerar.

**El estado de una punta `abstract` es `OPEN`**: constante, siempre sana, nunca pide acción. No hay contra qué compararla, así que no puede tomar otro valor. Se le da un nombre en vez de dejar el slot vacío porque la tupla `(state.0, state.1)` la consumen `check` para su código de salida, `accept .` para elegir a quién aceptar, `status` para imprimir y lattice para el campo `state` de la arista: un valor constante lo maneja cada uno sin ramas, un hueco obliga a todos a tratar el caso nulo. Es la misma forma que `TODO`. `accept .` nunca la toca.

**`abstract` es palabra reservada**, y con el prefijo de tipo de [ADR-0003](0003-formato-captures-y-aceptacion.md) no hace falta ninguna regla de desempate: es la única forma sin valor, y ninguna otra se le parece.

### 2. El endpoint repo

El UUID es el mismo que el del bilink remoto, así que no se escribe; y el otro repo se nombra por un alias local:

```yaml
# retinar/.bilink/<uuid>.yaml
endpoint:
  0:
    link: repo hsi
    accepted:
      link: 8f2a4c6e…      # id del capture del proveedor — ubicación publicada
      hash: c4e1770b…      # hash del fragmento del proveedor — contenido publicado
  1:
    link: capture 3d9b7e15-2a4c-4b6d-8e0f-1a2b3c4d5e6f
    accepted:
      link: capture 3d9b7e15-2a4c-4b6d-8e0f-1a2b3c4d5e6f
      hash: a71f5c3d…
```

**Se nombra con el prefijo `repo`**, sin sigilo. [ADR-0003](0003-formato-captures-y-aceptacion.md) vuelve explícito el tipo de todo endpoint, así que acá no hay nada que discriminar por forma: el valor arranca con la palabra y el resto es el alias.

Eso da de baja dos cosas que este ADR llegó a decidir. Un **separador sigilado** —se evaluaron `|` y `@`— buscando uno que no chocara con los paths Stratum ni necesitara comillas en la shell; y una **precedencia del parser** que desempatara entre prefijos, palabras reservadas y paths. Las dos existían porque el endpoint layer era el fallback. Sin fallback, el problema no existe.

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

El `.toml` declara la rama **del proyecto**, y la herramienta traduce a su ref de bilinks ([ADR-0004](0004-bilinks-en-ref-paralela.md)): `branch = "rc-2.32"` se busca en `refs/bilink/rc-2.32`. Una sola fuente de verdad, y la traducción es trabajo de bilinker. Como esa ref lleva el árbol del proyecto más `.bilink/`, un solo fetch trae las declaraciones del proveedor y el código al que apuntan, coherentes entre sí.

**Lo que gana la indirección por alias**: el `.bilink` no contiene ninguna URL, sólo un nombre local. Toda la identidad del proveedor —dónde está, qué rama— queda concentrada en un archivo por proveedor, que es el único lugar del consumidor que sabe algo del otro repo. Si `hsi` cambia de host, se edita un archivo y no N bilinks.

#### La frontera hereda la misma forma

El consumidor copia los **dos** valores aceptados del bilink remoto, igual que un endpoint layer copia el valor estructural de su vecino:

```yaml
# retinar/.bilink/<uuid>.yaml
endpoint:
  0:
    link: repo hsi
    accepted:
      link: 8f2a4c6e…      # id del capture del proveedor — ubicación publicada
      hash: c4e1770b…      # hash del fragmento del proveedor — contenido publicado
  1:
    link: capture 3d9b7e15-2a4c-4b6d-8e0f-1a2b3c4d5e6f
    accepted:
      link: capture 3d9b7e15-2a4c-4b6d-8e0f-1a2b3c4d5e6f
      hash: a71f5c3d…
```

Dos SHA-256 opacos: ninguno revela path, query, texto ni commit del proveedor. Se comparan por separado, así que la frontera obtiene el mismo vocabulario que el caso local **sin abrir el clon**:

| Comparación | Significado |
|---|---|
| las dos iguales | `OK` |
| `accepted.link` cambió, `accepted.hash` igual | el proveedor **movió** el fragmento y lo aceptó — revisar |
| `accepted.hash` cambió | el proveedor **cambió** el fragmento — revisar |

Un `apply` del proveedor **sin aceptar** no mueve nada: es trabajo en curso, y el consumidor no tiene por qué enterarse. Ésa es la razón de copiar campos y no el hash del archivo `.bilink` remoto entero.

Cada valor cambia por una sola razón, y por ninguna otra: `accepted.hash` cuando cambia el contenido publicado, `accepted.link` cuando cambia su ubicación aprobada. Los dos son inmunes a etiquetas, comentarios y reordenamientos del archivo remoto. Y ninguno **invalida la referencia** cuando el proveedor evoluciona, porque `link.N` sigue apuntando al UUID durable y sólo se mueven los valores aceptados. Poner el id del capture en el *endpoint* dejaría la referencia colgada en cada cambio, sin distinguir "cambió" de "nunca existió"; ponerlo en la *aceptación* no.

**Fan-out gratis**: el proveedor tiene **un** archivo, y cada consumidor tiene el suyo con el mismo nombre en su propio repo. Ninguno sabe del otro; el proveedor no sabe de nadie.

El consumidor lee además `link.1` del remoto para verificar que **sigue siendo `abstract`**; si dejó de serlo, es `REJECTED`. Dos lecturas, dos hechos, cero normalización.

Ningún commit del proveedor se copia. Cuando hace falta ubicar el suyo, se descubre recorriendo su ref hacia atrás hasta que su `accepted` coincida con los aceptados — lo que § "Profundidad a pedido" describe.

**El pin de versión es la branch, declarada una sola vez.** `branch` vive en el `.toml` y no en cada bilink: todos los vínculos de `retinar` hacia `hsi` siguen la misma rama, y cambiar de `main` a `rc-2.32` es editar una línea. Combinado con `bilinker/architecture.md` § "Implementaciones alternativas por branch", soportar dos versiones del proveedor a la vez son dos branches del consumidor, cada una con su `.hsi.toml` — no un campo de rango de versiones. La branch dice **qué línea seguir**; los valores aceptados dicen **qué vi en esa línea**.

#### Clona bilinker, y el sparse lo calcula, no lo declara

`sublayer-config.md` define `sparse` como un campo que el humano escribe, y acá eso sería un valor derivado metido en un archivo de declaración — el mismo error que ADR-0003 corrige en todos los demás archivos. El conjunto de archivos a traer sale de los bilinks:

1. Clonar sólo `.bilink/` — alcanza para el paso siguiente.
2. Por cada bilink local con endpoint repo hacia el alias, resolver `<clon>/.bilink/<uuid>.bilink`, seguir su endpoint estructural hasta `<clon>/.bilink/capture/<id>.capture`, y leer su `file`.
3. Ampliar el sparse-checkout a ese conjunto (`git sparse-checkout set`), sin volver a clonar.

No hace falta persistir el conjunto: git ya lo guarda en el clon, que además está gitignoreado. Y es **incremental por naturaleza** — sumar un vínculo de frontera agrega un archivo, sacarlo lo quita. Un conjunto fijo en el `.toml` quedaría desactualizado con el primer bilink nuevo.

Hacen falta los archivos y no sólo los `.bilink` porque detectar el drift y **entenderlo** son cosas distintas: los valores aceptados dicen que algo cambió, pero para mirar el fragmento, correr `get` y decidir si se acepta hay que tener el archivo del proveedor.

#### Profundidad a pedido

El clon arranca superficial: el árbol actual de la rama declarada, sin historia. Alcanza para `check`, que corre sobre todo y no puede andar profundizando clones como efecto colateral. Cuando alguien pide ver qué cambió entre lo aceptado y lo actual de **un** bilink, recién ahí se trae lo necesario: se recorre el `.bilink` remoto hacia atrás hasta la versión cuyo `accepted` coincide con los aceptados, se lee el capture de entonces, y se compara. Git lo soporta directo —`fetch --deepen`, o un clon parcial con `--filter=blob:none` que trae los blobs sólo cuando se los toca— y el costo se paga únicamente donde hay un humano mirando. Más adelante puede haber una flag tipo `--all` que traiga todo de entrada.

Ése es el reparto correcto: **`check` es masivo y barato; ver el diff es puntual y caro.** El conocimiento mínimo queda como default, no como límite — y lo que la profundidad a pedido acota es la *historia*, no el alcance: el sparse-checkout ya entrega el archivo entero del proveedor, que es más que el fragmento. Traer el archivo completo es, además, lo que a futuro —no definido acá— dejaría a lattice recorrer las relaciones de código que salen del fragmento y evaluar impacto a profundidad N.

**Del otro lado no hay capas.** Un alias nombra un repo, no una capa: bilinker busca el `.bilink/` de la raíz del clon y nada más. La estructura interna del proveedor —si tiene `.stratum/`, cuántas capas— no es asunto del consumidor, y meterla en el `.toml` sería volver a saber de más.

#### Taxonomía de ausencia

`check` **no clona nunca** — es masivo y offline. `LAYER_UNREACHABLE` significa *"declarada y ausente; el arreglo es clonarla"*; si el clon falla, eso es un error de `stratum pull`, no un estado de bilink.

| Situación | Estado | Arreglo |
|---|---|---|
| `.toml` presente, directorio ausente | `LAYER_UNREACHABLE` | `stratum pull` |
| Ni `.toml` ni directorio, con aceptación previa | `LAYER_UNCONFIGURED` | declarar la capa · o `bilinker remove` |
| Ni `.toml` ni directorio, sin aceptación previa | `TODO` | crear la capa |
| Contenedor presente, `.bilink` del UUID ausente | `BROKEN` | investigar — es regresión |
| Repo ajeno declarado, clon ausente | `REMOTE_UNREACHABLE` | lo clona bilinker |

`UNREACHABLE` a secas **desaparece del vocabulario**. El desdoblamiento se completa sin agregar un nombre de más, y `BROKEN` conserva su significado actual de `bilink.md` sin competir con nada.

Bajo un solo nombre no se puede distinguir "me falta traer algo" de "algo se rompió", que es la diferencia que decide si alguien tiene que mirar. Y no basta con separar en dos: **la subcapa y el proyecto ajeno también se arreglan con comandos distintos**, así que comparten poco más que la causa. Las dos ausencias que se arreglan trayendo algo son normales —`sublayer-config.md` ya dice que trabajar sin clonar todas las capas es lo esperado—, `LAYER_UNCONFIGURED` pide una declaración o un `remove`, y sólo `BROKEN` es una regresión.

El nombre sale de qué es la cosa, no de cómo se la trae. Una capa es una capa aunque su `.toml` la busque por URL; un proyecto git ajeno es, en el vocabulario de git, un remoto. No sirve `REPO_`, porque una capa Stratum **también** es un repo —`layer-model.md`: "cada proyecto y cada capa interna es un repositorio git independiente"— así que no contrastaría con nada. Tampoco el `external` de lattice, que nombra un URI http inverificable con garantía `asserted`: un repo ajeno se clona, se hashea y se verifica igual que lo propio.

`LAYER_UNREACHABLE` y `LAYER_UNCONFIGURED` son parte de esta decisión aunque no sean parte de la frontera: aplican al endpoint layer y aparecen sin que haya ningún proyecto ajeno de por medio. Dejarlos afuera dejaría el desdoblamiento a medias y obligaría a volver sobre las mismas tablas de estado dos veces. `sublayer-config.md`, que hoy dice que una subcapa no clonada reporta `UNREACHABLE`, se corrige acá.

`REJECTED` cuando el `link.1` remoto deja de ser `abstract`. La otra punta ya no admite ser ampliada, así que el vínculo no puede sostenerse — es un hecho distinto de "el fragmento cambió" y no debe mezclarse en el mismo token. El nombre describe la condición desde el lado que la sufre, que es el único que la puede observar: el proveedor no rechaza a nadie en particular, ni sabe que hay alguien.

**Conocimiento unidireccional.** El consumidor nombra al proveedor, y eso es inevitable y correcto: hay que saber de qué se depende. Lo que importa es que el proveedor no sabe de nadie. Y el requisito del Contexto —*"ningún extremo debe conocer al otro, ni su hash de contenido ni su ubicación"*— se cumple en su sentido operativo: lo que el consumidor guarda son dos SHA-256 de los que no se reconstruye nada, y el proveedor no guarda absolutamente nada del consumidor. La asimetría es el punto: es lo que disuelve el bloqueo 2, donde el hash copiado obligaba a los **dos** lados a saber del otro.

**Esto enmienda un principio.** `configuration.md:41` dice hoy "No existe `.bilinker.toml` ni ningún otro archivo de configuración". Deja de ser cierto, y conviene precisar qué cambia y qué no: la raíz se sigue descubriendo caminando hacia arriba, y el lenguaje se sigue infiriendo de la extensión — no aparece configuración *de la herramienta*. Lo que aparece es una declaración *del proyecto* sobre de qué depende, que es contenido, igual que un `.stratum/.impl.toml`. Pero la frase absoluta hay que reescribirla.

**Red, dicha con precisión.** Sólo git y tree-sitter. `check` opera completamente offline y **nunca hace red**: si el repo ajeno no está clonado, reporta `REMOTE_UNREACHABLE` y sigue. Las operaciones de red son explícitas y puntuales: el clon inicial del proveedor, el fetch de su ref, y la profundización de `get --diff`. `check.md:7`, que hoy dice "opera completamente offline" a secas para toda la herramienta, se corrige con esa distinción.


---

## Consecuencias

**Invariantes nuevas.** Un endpoint `abstract` no tiene ni va a tener contraparte en su propio repo · un endpoint repo se nombra con el prefijo `repo` más un alias, y el alias se resuelve por un `.bilink/.{alias}.toml`.

**Invariantes enmendadas.** La aridad sigue en dos: `abstract` es un valor, no una ausencia · una cadena tiene exactamente dos terminadores, cada uno un tip estructural, un `abstract` o un endpoint repo, y sigue siendo lineal: la bifurcación ocurre *entre* cadenas, en el UUID del bilink abstracto · **cae "no existe ningún archivo de configuración"**, aunque la raíz y el lenguaje se sigan resolviendo sin ella · "`check` opera completamente offline" pasa a ser cierto de `check` y no de toda la herramienta.

**Se va.** `UNREACHABLE` del vocabulario.

**Estados nuevos.** `OPEN` (punta `abstract`, constante y sana), `REJECTED` (la otra punta dejó de ser abstracta), `REMOTE_UNREACHABLE` (el repo del proveedor no está clonado), `LAYER_UNREACHABLE` (la subcapa está declarada y no clonada) y `LAYER_UNCONFIGURED` (ni declarada ni presente, con aceptación previa). `UNREACHABLE` a secas desaparece: sus tres situaciones quedan repartidas entre `LAYER_UNREACHABLE`, `REMOTE_UNREACHABLE` y `BROKEN`, que es lo que ya significaba en `bilink.md`. `LAYER_UNCONFIGURED` no sale de ahí sino de `BROKEN`: hoy `check_layer` devuelve `Broken` para toda capa ausente con aceptación previa, sin mirar si está declarada.

**Efecto por comando.** `check` gana rama nueva para el endpoint repo · `get --diff` cruzando la frontera profundiza el clon a pedido · `graph` emite `abstract` y repo como terminadores · `chain new` acepta un tip `abstract`.

**Limitación conocida.** Con el UUID implícito, un consumidor no puede vincular **dos** fragmentos locales a la misma abstracción remota: el nombre de archivo colisiona. Haría falta una segunda abstracción del lado del proveedor, y ahí el bloqueo 1 reaparece parcialmente. Se acepta, porque los consumidores reales ya centralizan el consumo en un único método por operación.

**Trabajo que dispara en las specs.** Reescribir `concepts/configuration.md`; ajustar los comandos `get` (`--diff` cruzando la frontera y la profundización del clon), `graph` (`abstract` y repo como terminadores) y `chain` (`chain new` con tip `abstract`); agregar `scenarios/frontier.yaml`. En Stratum, `sublayer-config.md`, que hoy reporta `UNREACHABLE` para una subcapa no clonada y declara `sparse` como campo escrito a mano — son 9 cadenas. En lattice —7 cadenas—: el nodo del bilink abstracto, las aristas dirigidas consumidor → abstracción, y **namespace de proyecto en el id canónico**, porque hoy `<layer-root>::<path>#<range>` es relativo a la raíz y dos proyectos con una capa en `.stratum/impl` colisionan.

**No hace falta migración.** Los endpoints `abstract` y repo son aditivos: ningún archivo existente los usa, y todos siguen siendo válidos. La frontera se puede adoptar bilink por bilink.

---

## Alternativas descartadas

- **URL de git inline en el endpoint** (`link.0: git@… rc-2.32`). Directo y sin archivo aparte, pero mete la identidad del proveedor en cada bilink y obliga a editar N archivos para cambiar de rama o de host.
- **Un separador sigilado para el endpoint repo** — se evaluaron `|` y `@`. `|` se eligió primero creyendo que ningún path Stratum válido empieza así, y es falso: `paths.md` define `traditional-path` como cualquier path relativo que no empiece con `>` ni `<`. `@` lo reemplazó por no necesitar comillas en la shell. Los dos quedaron sin sentido cuando [ADR-0003](0003-formato-captures-y-aceptacion.md) volvió explícito el tipo de todo endpoint: con `repo hsi` no hay nada que discriminar por forma, y de paso el valor deja de empezar con un sigilo, que en YAML habría exigido comillas.
- **UUID explícito en el endpoint repo.** Permitiría que el consumidor elija su propio UUID y levanta la limitación conocida, al costo de un token redundante y de romper la convención de UUID compartido que ya rige las cadenas.
- **Hashear el archivo `.bilink` remoto entero** en vez de copiar sus campos aceptados. Dispara con el `apply` sin aceptar del proveedor —trabajo en curso que el consumidor no puede resolver—, exige definir una forma canónica porque el formato admite comentarios y no fija orden de claves, y colapsa `REJECTED` en el mismo token.
- **Manifiesto publicado con token de revisión** (`contract publish` / `fetch`, `rev` `<mayor>.<serial>`, clasificación breaking/compatible declarada). Da semántica de compatibilidad, pero es maquinaria nueva entera para conseguir tokens opacos que el `accepted` remoto ya da derivado en vez de declarados.
- **Sólo captures del lado del proveedor.** Un capture no tiene estado de aceptación, así que el proveedor no podría responder si lo publicado sigue coincidiendo con lo aprobado.
- **`link.1` ausente en vez de `abstract`.** Rompe la aridad fija en dos por un campo vacío.
- **`sparse` declarado en el `.toml`.** Es un valor derivado de los bilinks del proyecto, y quedaría viejo con el primer vínculo nuevo.
- **El capture guarda el texto aceptado.** Lo volvería auto-contenido y sin arqueología de git, pero **un chunk congelado adentro del `.capture` es una copia sin procedencia**: no se verifica, no se navega, no se refresca. Un checkout real son objetos git verificables que se actualizan en el fetch. (No se rechaza por "conocimiento máximo": el sparse-checkout ya entrega el archivo entero, que es más.)
- **Raíz de ecosistema**, un repo contenedor con los tres proyectos como sub-proyectos. No resuelve los bloqueos 1–3.
