# La verificación

`check` verifica la consistencia de uno o más bilinks y no escribe ni un byte en git.

Opera en dos pasos: resuelve los captures referenciados —localizando cada fragmento en el árbol actual— y compara lo hallado contra `accepted`, en sus dos dimensiones: dónde está y qué dice. El resultado va a [`cache/state`](cache.md).

## Qué necesita y qué no

### Git y tree-sitter le alcanzan para todo menos un eje

Requiere git como dependencia dura. El [vecindario](accept.md) se verifica en dos partes, y sólo una necesita resolver tipos:

| Qué se verifica | Con qué |
|---|---|
| que las declaraciones de los vecinos aceptados no cambiaron | sus captures, con tree-sitter |
| que los nombres de la firma sigan resolviendo a esos vecinos | el daemon de `lspd`, por el puerto del vecindario |

La segunda depende de los imports del archivo, de lo que declara el mismo paquete y de las dependencias del build, que están fuera del fragmento y fuera de los captures. Por eso la pregunta entra por un puerto que el binario implementa contra un language server.

Una capa sin ningún nivel 1 adquirido —sin `n`, o con todo `declined`— no le pregunta a nadie, y `check` corre ahí con git y tree-sitter solos.

### `check` usa el daemon activo y nunca lo levanta

Levantar y apagar el daemon es un paso explícito, fuera de los comandos: `lspd start --wait` y `lspd stop`. Un comando que arranca un daemon deja el apagado sin dueño.

Si la capa tiene algún nivel 1 adquirido y el daemon no contesta, `check` falla antes de verificar nada, sale con 2, y nombra las dos salidas con los lenguajes que hacen falta:

```
error: 108 endpoint(s) tienen nivel 1 y no hay daemon en esta capa.
  Levantarlo:       lspd start --wait --lang java --lang typescript
  Sin confirmarlo:  bilinker check . --no-ask-n1
```

Un daemon que indexa se espera, lo haya levantado quien sea, con la regla de [la aceptación](accept.md). Uno que falla hace fallar `check` con 2: no terminó de verificar. Lo que alcanzó a verificar antes queda en la cache, como en cualquier check parcial.

### `--no-ask-n1` verifica lo que alcanzan git y tree-sitter, y dice lo que no confirmó

Con `--no-ask-n1`, `check` no le pregunta a nadie en esta corrida. Verifica el fragmento y el contenido de los vecinos aceptados, y lo único que queda sin confirmar es la resolución de los nombres, que es `OK_N1_UNCONFIRMED`.

Es lo que declara una vez un CI sin language server. Nunca baja cobertura: no escribe nada, y bajarla es `accept --decline-n1`.

### `check` opera completamente offline

Es de `check` y no de toda la herramienta. Es masivo: corre sobre todos los bilinks de una capa, así que no puede clonar ni fetchear como efecto colateral. Un repo ajeno que no está clonado se reporta `REMOTE_UNREACHABLE` y se sigue. Las operaciones de red viven en otros comandos y son explícitas: el clon de un proveedor, el fetch de su ref, y la profundización de [`get --diff`](get.md).

### `check` toma un bilink o una capa, y con `--against` una ref

```
bilinker check [<path>] [--against <ref>] [--no-ask-n1]
```

| Argumento | Descripción |
|---|---|
| `path` | Path a una capa, a un bilink individual, o a un archivo o directorio de la capa. Default: la capa actual. |
| `--no-ask-n1` | No le pregunta al daemon por el nivel 1: lo que no confirma es `OK_N1_UNCONFIRMED`. |
| `--against <ref>` | Toma los `accepted` de otro lado en vez de los del árbol, y no escribe cache. |

### Un path que no es una capa ni un bilink verifica lo que cae bajo él

El alcance sale del path, en este orden:

| El path | Qué se verifica |
|---|---|
| tiene `.bilink/` adentro | la capa entera |
| es un `.yaml` de `.bilink/` | ese bilink |
| cualquier otro archivo o directorio | los bilinks con algún endpoint cuyo capture tiene su `file` bajo ese path |

Es la pregunta de quien está tocando un archivo: *"¿qué dice lo que está atado a esto?"*. Con la capa entera la respuesta llega mezclada con todo lo demás, y en una capa con cientos de bilinks eso es no contestarla.

Lo que cuenta es el `link` de cada endpoint, la ubicación vigente. Un vecino del [vecindario](accept.md) no mete a su bilink en el alcance: el archivo de un DTO no es el fragmento de nadie, y un bilink se verifica igual entero cuando entra.

Un path que no existe es un error y no un alcance vacío. Con un typo, *"no hay nada no-OK"* se leería como que todo está bien.

### Un check parcial sólo escribe en la cache lo que verificó

Los estados del resto de la capa quedan como estaban. Borrarlos haría que `status`, que lee la cache sin verificar, dejara de mostrar bilinks que nadie tocó sólo porque se preguntó por otro archivo.

### `--against` compara contra las aceptaciones de otra parte

Otra rama, otro commit, sin tocar nada. Sirve para preguntar *"¿qué endpoints quedarían no-OK si mergeo esto?"* antes de mergearlo.

No escribe cache a propósito: la cache describe el estado del árbol contra sus propias aceptaciones, y sobrescribirla con el resultado de una comparación hipotética la volvería mentirosa.

Lo que `--against` no puede hacer es cruzar versiones de formato: linkea un solo parser. Comparar dos formatos es trabajo de una migración, que depende de los dos.

## Antes del primer bilink, la versión de la capa

### La versión se compara antes de abrir un archivo

`.bilink/version` dice en qué formato están los archivos de esta capa. `check` la compara contra la versión de su propio parser antes de abrir uno, y si no la entiende no verifica nada: dice qué versión hay, qué versión lee, y manda a `migrate`.

```
$ bilinker check .
Error: esta capa declara formato 3.0.0 y este binario lee 4.0.0.
       No se interpreta lo que no se entiende: bilinker migrate --recursive
```

Es el criterio que ya usa cruzando la [frontera](frontier.md), aplicado del lado de casa: la misma comparación de major, el mismo *"no se interpreta lo que no se entiende"*. Una versión que no se entiende no es un estado de los bilinks, es no poder leerlos, y reportar cualquier estado sobre eso sería inventar. El que malinterpreta es el parser, y no le cambia nada de quién sean los archivos.

Y no sirve deducirlo del parseo, porque un archivo de formato viejo puede parsear bien y significar otra cosa. En `3.3.0` la `query` de un capture pasó a poder llevar varios `@target` y el fragmento pasó a ser su concatenación, sin que el tipo ni el archivo cambiaran: un parser de `3.2.0` lee esa query, se queda con el primero, y hashea otro fragmento sin fallar. La versión es lo único que discrimina en esa dirección, así que se pregunta primero.

### La versión importa cuando hay archivos que leer

Hay tres situaciones y sólo una es un problema:

| En disco | Qué es | `check` |
|---|---|---|
| no hay `.bilink/`, o hay y sólo tiene la cache y su `.gitignore` | no hay archivos del formato que malinterpretar | `all clean (0 bilink(s))`, y es cierto |
| hay bilinks o captures, y `version` es de este major | se puede leer | verifica |
| hay bilinks o captures, y `version` es de otro major o no está | un formato que este binario no lee | se niega y manda a `migrate` |

El discriminador es que haya archivos del formato, no que exista el directorio. Cruzando la frontera el consumidor crea `.bilink/` sólo para poner el `.{alias}.toml`, antes de que exista un solo bilink; y una capa recién declarada tiene su cache y nada más. Negarse ahí sería negarse justo donde no hay nada que se pueda leer con el parser equivocado.

La tercera fila incluye la ausencia de `version` porque una capa con archivos y sin declaración es formato 1 —anterior a que el campo existiera—, que es la misma lectura que hace la frontera de un proveedor que no declara nada.

### Un binario más viejo que la capa falla al parsear, y eso ya está cubierto

La comparación es de major, así que una capa `4.1.0` leída por un binario `4.0.0` pasa este control y falla después, en el archivo que lleva el campo nuevo. `deny_unknown_fields` es explícito a propósito, y es exactamente lo que el [registro de versiones](format-version.md) existe para garantizar.

### Y crear la capa es declarar su formato

Declarar la versión es parte de escribir el primer archivo en una capa, y no un paso aparte: un paso aparte es un paso olvidable, y una capa nacida de un `capture` suelto sin `version` sería indistinguible de una anterior a que el campo existiera.

Con un límite: sólo si no hay nada que malinterpretar. Un `.bilink/` que ya tiene bilinks o captures y no declara nada es formato 1, y estamparle la versión de hoy escribiría una respuesta falsa encima de una verdadera; decidir eso es de `migrate`. Una capa vacía no tiene ese problema, y ahí declarar es lo honesto.

## Un archivo que no parsea es un estado, no una ausencia

### Un bilink ilegible se cuenta aparte, se imprime con su error y sale con 1

Saltear un bilink ilegible para no abortar el recorrido de los demás está bien. Lo que no está bien es saltearlo en silencio, porque ahí *"no pude leer 206"* sale igual que *"no hay ninguno"*: `all clean (0 bilink(s))` con código 0.

```
$ bilinker check .

3 bilink(s) no se pudieron leer:
  .bilink/7f3d8e9a-….yaml  unknown field `agree`, expected one of `link`, `hash`, `hash_ast`
  .bilink/3a4b5c6d-….yaml  invalid type: sequence, expected a map
  .bilink/f1e2d3c4-….yaml  missing field `link`

203 bilink(s) verificados, todos OK — 3 ilegibles
```

`all clean` es una afirmación sobre todo lo que hay, así que no se imprime cuando quedó algo sin leer. Un `check` que verificó 203 de 206 no puede decir `all clean (206)` ni `all clean (203)` a secas: el primero miente sobre lo que miró, el segundo esconde lo que no pudo mirar.

Y el conteo va al lado del resultado, no en vez de él. Los 203 que sí se leyeron se evalúan y se reportan igual: un archivo roto no es razón para dejar de decir lo que se sabe de los demás. Por eso la línea final aparece también cuando hubo no-OK —`203 bilink(s) verificados — 3 ilegible(s)`—: con algo sin leer, falta decir sobre cuántos se dijo.

## Las dos dimensiones

### La ubicación se decide siempre, y primero

Un endpoint puede desalinearse de dos formas, y `check` las distingue porque se aprueban por separado:

| | Se compara | Da |
|---|---|---|
| Ubicación | `link` contra `accepted.link` | `RELOCATED` |
| Contenido | el hash del fragmento contra `accepted.hash` | `ALTERED`, `RESTYLED`, … |

La de ubicación es una comparación de dos ids: no hace falta abrir ningún archivo. Por eso sobrevive donde la otra no —cruzando la frontera, con un clon superficial— y por eso se evalúa primero.

## Los estados

### Los estados de resolución del capture

Sobre dónde está el fragmento. Se evalúan sin ninguna aceptación.

| Estado | Condición | Fix |
|---|---|---|
| RESOLVED | La query matchea. | — |
| MOVED | El archivo cambió de path (git rename ≥ 50%). | `apply` |
| REANCHORED | Anchor renombrado; el fragmento se localizó bajo otro nombre por similitud. | `apply` |
| UNANCHORED | La query no matchea y el anchor no se localiza. | `recapture` |
| DELETED | Eliminación rastreable con `git log -S`. | intervención |
| BROKEN | Ninguna hipótesis aplica. | intervención |

### Los estados de aceptación

Comparan lo hallado contra `accepted`.

| Estado | Condición | Fix |
|---|---|---|
| PENDING | `accepted` ausente. | `accept` |
| OK | La ubicación y el contenido coinciden con lo aceptado. | — |
| RELOCATED | `link` ≠ `accepted.link`. | `accept --place` |
| EXPANDED | El fragmento contiene lo aceptado verbatim y algo más. | revisar + `accept` |
| RESTYLED | El texto difiere pero el AST coincide: sólo formato. Sólo donde el AST discrimina contenido. | `accept` |
| ALTERED | El fragmento cambió estructuralmente. | revisar + `accept` |
| UNRESOLVED | El capture referenciado no resolvió. | se resuelve en el capture |
| CONSENSUS_DIVERGED | Más de una entrada en `accepted`. | `accept` |
| CONTRACT_RESTYLED | El vecindario de la firma se reformateó. | `accept` |
| CONTRACT_ALTERED | Un tipo que la firma menciona cambió. | revisar + `accept` |
| CONTRACT_RELOCATED | El conjunto de vecinos declarado ≠ el aceptado. | revisar + `accept` |
| CONTRACT_UNLOCATED | El vecindario aceptado conserva su contenido y su ubicación es `unknown`. | acuñar sus captures + `accept` |
| OK_N1_UNCONFIRMED | Todo `OK` salvo la resolución de los nombres de la firma, que no se preguntó: `--no-ask-n1`. | `check` con el daemon |

`EXPANDED` necesita el texto aceptado, así que se detecta acá y no en la dimensión de ubicación.

La consistencia se evalúa por extremo: `check` retorna una tupla `(state.0, state.1)`, y cada extremo puede estar en un estado diferente: `(OK, MOVED)`, `(RELOCATED, ALTERED)`, `(OK, OK)`.

### `CONTRACT_UNLOCATED`: el contrato está y su ubicación no se sabe

El `link` de un nivel puede ser [`unknown`](bilink.md): los dos hashes conservados y ningún id. Ahí el eje de la ubicación no se puede comparar, y no poder compararlo no es que coincida.

No es ninguno de los dos estados vecinos, y la diferencia no es de matiz:

| | Qué dice |
|---|---|
| `CONTRACT_RELOCATED` | los dos conjuntos están y difieren |
| `OK_N1_UNCONFIRMED` | los vecinos aceptados no cambiaron, y nadie preguntó si la firma los sigue nombrando |
| `CONTRACT_UNLOCATED` | el contenido está aprobado y de qué vecinos salió no se sabe |

Sale con 1, y ahí está la diferencia que importa con `OK_N1_UNCONFIRMED`: éste no es una pregunta que no se hizo sino trabajo escrito en el archivo. Hay captures que alguien tiene que acuñar, y hasta que se acuñen el nivel no detecta que un vecino se mudó de archivo o se renombró.

Y sin daemon se contesta igual: comparar ids nunca lo necesitó. Es lo que garantiza que un nivel sin ubicación no desaparezca del inventario con `--no-ask-n1`.

### El contenido del nivel 1 se verifica con los captures de los vecinos

Cada id de `accepted.n.1.link` es un capture. `check` los resuelve por su query y los pliega con el mismo fold que calcula `accept`: el mismo orden por id, el mismo recorte de bordes y el mismo `hash_ast` todo-o-nada. Si el fold difiere de `accepted.n.1.hash`, un vecino cambió, y eso es drift probado: no necesita daemon, y sale igual con `--no-ask-n1`.

| El fold de los captures | Estado |
|---|---|
| igual | sigue a la ubicación y a la resolución |
| distinto en texto, igual en `hash_ast` | `CONTRACT_RESTYLED` |
| distinto | `CONTRACT_ALTERED` |

Un vecino cuyo capture ya no resuelve es `CONTRACT_ALTERED`: la declaración aceptada ya no está donde estaba, y eso es un cambio, no una ausencia.

Con el `link` del nivel en `unknown` no hay captures que resolver, y el contenido sólo se puede comparar recalculando el vecindario con el daemon.

### La resolución de los nombres la confirma el daemon

Que las declaraciones no cambiaron no dice que los nombres de la firma sigan resolviendo a ellas. Con el daemon, `check` le pregunta el conjunto de hoy y lo compara por ids contra el aceptado: si difiere, es `CONTRACT_RELOCATED`. Con el `link` en `unknown` no hay ids, y compara el fold del conjunto de hoy contra el `hash` conservado.

### `OK_N1_UNCONFIRMED`: todo lo que git y tree-sitter miran está bien, y la resolución no se preguntó

Sale sólo con `--no-ask-n1`, y sólo sobre un nivel 1 adquirido, también el vacío: un import nuevo puede meterle un vecino. Sin `n` o con `declined`, el estado es `OK`.

Sale con 0. No se lista uno por uno: el resumen dice cuántos son y qué correr para confirmarlos, y se listan filtrando la salida de [`status`](#muestra-la-cache-agrupada-por-archivo-sin-re-verificar) por el estado.

### Lo probado le gana a lo sospechado

Un endpoint tiene un estado. Sobre el nivel 1, en este orden:

1. `CONTRACT_ALTERED` y `CONTRACT_RESTYLED`: el contenido de un vecino cambió.
2. `CONTRACT_RELOCATED` y `CONTRACT_UNLOCATED`: el conjunto difiere, o no tiene ubicación.
3. `OK_N1_UNCONFIRMED`: no se preguntó.
4. `OK`.

El eje del vecindario se evalúa sólo cuando el del fragmento dice `OK`.

### Un cambio real de contrato le gana a la ubicación faltante

Con proveedor, el contenido del nivel se compara igual: se recalcula el vecindario y se pliega contra el `hash` conservado, que es todo el punto de haberlo conservado. Si difiere, el estado es `CONTRACT_ALTERED`.

Es la única prioridad posible entre los dos. Un endpoint tiene un estado y no dos, y de los dos candidatos uno ya está escrito en el archivo —que la ubicación falta se ve abriéndolo— y el otro no: que un vecino cambió sólo lo dice haber recalculado. Nombrar el que ya se puede leer taparía el que no.

Los dos salen con 1, así que la elección no cambia el inventario: cambia qué se lee primero sobre un endpoint que está en los dos estados a la vez.

### `RESTYLED` sólo existe donde el AST discrimina contenido

En prosa el AST no lleva el texto: el s-expression de una sección markdown es el mismo con cualquier párrafo adentro. Un paso de Gherkin es prosa del mismo modo: su árbol es la palabra clave y un texto libre. Comparar ahí diría *"sólo formato"* de una reescritura entera, que es exactamente el estado que invita a aceptar sin leer.

Así que la pregunta la decide la gramática, no el archivo: en markdown, Gherkin y texto plano `accept` no escribe `hash_ast` y `check` no lo compara. Los dos consultan la gramática antes que `accepted`, así que un `hash_ast` guardado por una versión anterior queda inerte en vez de mentir, y `accept` tampoco lo arrastra hacia adelante.

La lista de lenguajes donde el AST discrimina es la de [capture.md](capture.md), "Lenguajes soportados", menos markdown y Gherkin.

### `hash_ast` cubre los tokens, no sólo la forma del árbol

El s-expression de tree-sitter es la forma del árbol: dice `(identifier)`, no qué identificador. Hashear eso solo hace invisible todo renombre y todo literal: `("0.1.0", "21e2…")` y `("2.0.0", "3939…")` tienen el mismo árbol, y el estado saldría `RESTYLED` de un cambio de versión.

`hash_ast` es entonces la forma del árbol más el texto de cada token hoja. Dos fragmentos coinciden cuando tienen los mismos tokens en el mismo orden y la misma estructura; lo único que puede diferir es el espacio entre ellos, que es lo que *"sólo formato"* quiere decir.

Un comentario es un token, así que cambiarlo no es `RESTYLED`. Un comentario dice algo, y cambiar lo que dice es un cambio de contenido.

Un `hash_ast` calculado con una definición anterior simplemente no coincide, y el endpoint sale `ALTERED`, que pide revisión. No hace falta migrar nada.

### EXPANDED: creció alrededor de lo aceptado

Se distingue con un test de subcadena contra el texto aceptado, no con un umbral. Siendo `T` el texto aceptado y `F` el fragmento que el capture resuelve hoy:

| Condición | Estado |
|---|---|
| `F == T` | OK |
| `F ⊃ T`: contiene lo aceptado y algo más | EXPANDED |
| `T` no aparece y `hash_ast` coincide | RESTYLED |
| nada de lo anterior | ALTERED |

Que `F` contenga a `T` verbatim implica que nada dentro de lo aceptado cambió, así que la condición de *"AST interno sin cambio estructural"* se satisface sola.

Sin `T` no hay EXPANDED. El texto aceptado sale de git —el `commit` del contenido más el path del capture— y eso puede no estar. Cuando falta, la comparación por subcadena no se puede hacer y el estado cae en ALTERED, que pide revisión. Es la respuesta segura, y como el commit se re-deriva, el caso es raro.

### Por qué REANCHORED usa similitud y no el hash

`accepted.hash` es exacto, y el nombre del anchor está dentro del fragmento capturado en la enorme mayoría de los casos. Renombrar el anchor cambia el fragmento, así que una comparación por hash no dispararía nunca: detectaría solo el caso raro en que lo renombrado queda fuera de lo capturado.

El texto aceptado se recupera de git y se compara contra cada candidato.

Umbral: 50%, el mismo que usa `git diff -M` para renames de archivos. La pregunta es la misma —a dónde se fue algo que cambió de nombre— y usar dos criterios distintos para la misma pregunta sería arbitrario.

Margen sobre el segundo candidato: 15%. Un archivo con varias funciones de forma parecida produciría un REANCHORED arbitrario. Ante un empate el estado es `UNANCHORED`: que lo mire un humano es mejor que reanclar al nodo equivocado.

La medida es el coeficiente de Dice sobre líneas, con bigramas de caracteres como respaldo para fragmentos de una sola línea, donde las líneas no discriminan nada.

### La incertidumbre está acotada

Introducir una medida difusa en un sistema construido sobre hashes exactos necesita un límite claro, y lo tiene: `REANCHORED` nunca cierra solo. `apply` corrige la ubicación pero el endpoint queda no-OK hasta que un humano ejecute `accept`. La similitud sirve para encontrar el fragmento, nunca para afirmar que su contenido sigue siendo válido: eso lo sigue decidiendo un hash exacto.

### Estados propios de un endpoint `path`

| Estado | Condición | Fix |
|---|---|---|
| TODO | `accepted` ausente y la capa apuntada no existe todavía. | crear la capa + `accept` |
| CHAIN_DIRTY | Los valores copiados ≠ los `accepted` del vecino. | `accept` |
| LAYER_UNREACHABLE | La capa está declarada y no clonada. | `stratum pull` |
| LAYER_UNCONFIGURED | Ni declarada ni presente, con aceptación previa. | declarar la capa · o `remove` |
| BROKEN | La capa ya no existe, o el vecino no tiene endpoint estructural aceptado. | restaurar + `accept` · o · `remove` |

`TODO` indica una intención declarada, no un error: se resuelve creando la capa destino y ejecutando `bilinker accept`. Si la capa existía —el endpoint tiene `accepted`— pero desapareció, el estado es `BROKEN` (regresión).

`CHAIN_DIRTY` no tiene auto-fix directo: se resuelve ejecutando `bilinker accept` en el endpoint `path`. Esto evita dependencia circular: aceptar un endpoint `path` nunca modifica el archivo adyacente, así que no hay cascadas. La propagación es unidireccional desde el endpoint estructural que cambió.

## Las partes del contenido

### Con dimensiones, el estado del contenido sale de ellas

Un endpoint que declara [dimensiones](bilink.md#las-dimensiones-parten-el-contenido-del-fragmento) no compara el fragmento entero: compara cada parte contra lo que se aprobó de ella, por nombre. `accepted.hash` sigue siendo el del fragmento, y deja de decidir el estado. Lo que cambia afuera de toda parte declarada no avisa, porque nadie pidió vigilarlo.

Cada dimensión se compara como un fragmento, con la tabla de [EXPANDED](#expanded-creció-alrededor-de-lo-aceptado) y las mismas condiciones para `RESTYLED`:

| La parte | Estado de la dimensión |
|---|---|
| hashea a lo aprobado | `OK` |
| contiene lo aprobado verbatim y algo más | `EXPANDED` |
| difiere en texto y coincide en `hash_ast` | `RESTYLED` |
| nada de lo anterior | `ALTERED` |
| está de un solo lado: declarada y no aprobada, o aprobada y no declarada | `ALTERED` |
| su query no encuentra la parte | `ALTERED` |

La ubicación se decide antes, como siempre: un `RELOCATED` no mira ninguna parte. Y el vecindario se evalúa sólo cuando todas las partes dicen `OK`.

Un endpoint sin dimensiones compara el fragmento entero, igual que antes de que existieran.

### Una dimensión se busca adentro del nodo del capture

La query de la dimensión se evalúa sobre el archivo y se queda con el primer match cuyos `@target` caen todos adentro del fragmento que resolvió el capture. Así una parte nunca sale de otro método del mismo archivo.

Una parte que está afuera del nodo —la anotación de la clase que contiene al método— hoy no se encuentra, y su dimensión da `ALTERED`.

### La dimensión califica al estado, y no lo reemplaza

El estado de un endpoint sigue siendo una palabra, y del vocabulario de siempre. Las dimensiones que no están `OK` van al lado, entre paréntesis y por nombre: `ALTERED(body)`, `ALTERED(parameters, route)`.

La palabra es lo que consumen el filtro por estado, el código de salida y el agrupado de `status`, y por eso no cambia. Los nombres de dimensión son del generador: un estado por dimensión dejaría el vocabulario abierto, y un filtro no podría validar nada contra él.

### Varias dimensiones dan la palabra más severa

Cuando difiere más de una parte, la palabra es la de la más severa, en este orden:

1. `ALTERED`: pide revisar, y falla.
2. `EXPANDED`: pide revisar, y no falla.
3. `RESTYLED`: sólo pide aceptar.
4. `OK`.

Y el reporte las lista a todas, cada una con su estado. Un endpoint con el cuerpo sólo reformateado y la ruta cambiada es `ALTERED(body, route)`, y la línea del endpoint dice cuál es cuál:

```
$ bilinker check .

3a4b5c6d  (ALTERED(body, route), OK)
  endpoint.0  body RESTYLED · route ALTERED
```

`body` está en la calificación aunque no haga fallar a nadie: aceptar el endpoint aprueba todas las partes juntas, así que quien revisa tiene que saber que también cambió.

## Recuperar el texto aceptado

### El texto aceptado se resuelve con la query contra el contenido del commit

Dos detecciones —EXPANDED y REANCHORED— necesitan el texto del fragmento tal como quedó aceptado, no solo su hash. Se recupera de git:

```
git show <commit>:<file>   →  contenido en el momento de aceptar
ejecutar la query sobre él   →  el nodo
resolver la query            →  el fragmento aceptado
verificar sha256 == accepted.hash   →  o descartar
```

No se recorta por el `range` cacheado. `check` lo reescribe en cada corrida, así que apunta a dónde está el fragmento ahora; recortar contenido viejo con una posición nueva da bytes arbitrarios. Resolver la query contra el contenido viejo es lo correcto, y además se autoverifica.

Si la verificación falla —no hay `commit`, el archivo no existía en ese commit, la query no resuelve ahí— el texto se descarta y esas detecciones no corren: el estado cae en ALTERED, que pide revisión. Es preferible no distinguir que razonar sobre el texto equivocado.

### No hay optimización por diff de git

`check` no conserva un `state.N` de `OK` porque el archivo no haya cambiado desde el commit del contenido aceptado. La pregunta que hay que contestar es *"¿el fragmento sigue hasheando a `accepted.hash`?"*; la que ese atajo contesta es *"¿cambió el archivo?"*. Las dos coinciden mientras el fragmento se derive del archivo de la misma manera, y dejan de coincidir apenas cambia cómo se resuelve el rango: ahí el mismo archivo produce otro fragmento, el archivo no se tocó, y el atajo devuelve `OK` para siempre.

Pasó al cambiar los bordes del rango en una migración: diecisiete endpoints de accreta quedaron con un `accepted.hash` que ya no describía lo que había, y `check` los reportó `OK`, con `accept` creyéndole y no aceptando nada. Quedaron invisibles hasta que alguien borró la cache.

Y no compra nada. Lo que ahorra es leer un archivo y hashearlo; lo que gasta es un subproceso de git. Medido sobre accreta, las dos cosas cuestan lo mismo.

Un estado se conserva verificándolo, no infiriéndolo de que su entrada no cambió, porque *"su entrada"* incluye a la herramienta, y la herramienta también cambia.

## Algoritmo de detección por tipo de endpoint

### Endpoint estructural

Los pasos 1–2 resuelven el capture; los pasos 3–8 comparan contra `accepted`.

```
1. ¿El archivo existe en el path conocido?
   NO → git diff -M --name-status HEAD
        ¿rename ≥ 50%?
        SÍ → MOVED
        NO → git log -S "<accepted.hash>" -- <file>
             SÍ → DELETED
             NO → BROKEN

2. Ejecutar query tree-sitter.
   SIN MATCH → relajar la query (quitar los predicados #eq?) y puntuar
               cada candidato por similitud contra el texto aceptado:
               ¿el mejor supera el umbral y le saca margen al segundo?
               SÍ → REANCHORED
               NO → git log -S "<accepted.hash>" -- <file>
                    SÍ → DELETED
                    NO → UNANCHORED

   (los pasos 1–2 son del capture; si no resuelve, todos los endpoints
    que lo referencian quedan UNRESOLVED y se corta acá)

3. ¿accepted ausente?  → PENDING
4. ¿link ≠ accepted.link?  → RELOCATED     ← ubicación: dos ids,
                                              sin abrir ningún archivo
5. ¿Hash matchea en el range?  → OK
6. Recuperar el texto aceptado T (ver "Recuperar el texto aceptado").
   ¿F contiene T verbatim y es más grande?  → EXPANDED
7. ¿accepted.hash_ast presente y el hash_ast actual coincide?
   SÍ → RESTYLED  (sólo espaciado; mismos tokens)
8. → ALTERED
```

El paso 4 va antes que el 5 y no cuesta nada. Comparar dos ids no abre ningún archivo, así que la dimensión de ubicación se decide siempre, incluso cruzando la frontera, donde el clon superficial no permite recuperar el texto aceptado y la de contenido degrada a `ALTERED`.

Un mismo capture se resuelve una sola vez por `check`, aunque lo referencien varios endpoints. Los pasos 3–8 sí corren por endpoint, porque cada uno tiene su propio `accepted`.

Con más de una entrada en `accepted`, el estado es `CONSENSUS_DIVERGED` sin evaluar los otros ejes: no hay un valor contra el cual compararlos.

### Endpoint `path`

```
1. Resolver path: ../<stratum-path>/.bilink/<uuid>.yaml
2. ¿La capa o el archivo no existen?
   .toml presente, directorio ausente        → LAYER_UNREACHABLE
   ni .toml ni directorio, accepted ausente  → TODO
   ni .toml ni directorio, accepted presente → LAYER_UNCONFIGURED
   capa presente, .bilink del uuid ausente   → BROKEN
3. Leer el `accepted` del endpoint estructural del bilink adyacente.
   ausente → PENDING (el otro extremo nunca se aceptó)
4. ¿accepted propio ausente? → PENDING
5. Comparar las dos copias guardadas contra las del vecino.
   las dos coinciden → OK
   alguna difiere    → CHAIN_DIRTY
```

Se comparan dos valores, no uno: `accepted.link` y `accepted.hash`. Cada uno cambia por una sola razón —la ubicación aprobada del vecino, o su contenido aprobado— y los dos son inmunes a etiquetas, comentarios y reordenamientos de su archivo.

El paso 2 distingue tres ausencias, no una ([frontier.md](frontier.md), "Taxonomía de ausencia"): las dos primeras se arreglan trayendo o declarando algo y son normales; sólo `BROKEN` es una regresión.

### Endpoint `abstract`

```
→ OPEN
```

No hay contra qué comparar. Constante, sana, y `accept .` nunca la toca.

### Endpoint repo

Es el endpoint `path` con la dirección resuelta por alias, y sin red:

```
1. Resolver el alias: .bilink/.{alias}.toml → .bilink/{alias}/
2. ¿El clon no está?  → REMOTE_UNREACHABLE   (no se clona: check no hace red)
3. Verificar el .bilink/version del clon.
   no se entiende → error, no un estado: se para en vez de malinterpretar
4. Resolver <clon>/.bilink/<uuid>.yaml
   ausente → BROKEN
5. ¿El link de la otra punta del bilink remoto sigue siendo `abstract`?
   NO → REJECTED
6. Leer el `accepted` de su endpoint estructural.
   ausente → PENDING (el proveedor nunca aceptó lo que publica)
7. ¿accepted propio ausente? → PENDING
8. Comparar las dos copias guardadas contra las del proveedor.
   las dos coinciden → OK
   alguna difiere    → CHAIN_DIRTY
```

El paso 3 no devuelve un estado. Una versión de formato que no se entiende no es drift: es no poder leer los archivos, y reportar cualquier estado sobre eso sería inventar. Entre proyectos con releases independientes la divergencia de versiones es lo normal, no un accidente.

El paso 5 va antes que el 8, y es la razón de leer dos cosas del remoto y no una: que la punta dejó de ser `abstract` es un hecho distinto de que el fragmento cambió, y mezclarlos en el mismo token perdería cuál de los dos pasó.

Nada de esto abre un archivo del proveedor: los pasos 5 a 8 leen su `.bilink`, que el clon superficial ya trae. Mirar el fragmento es trabajo de [`get`](get.md), y ahí sí puede hacer falta profundizar.

### Endpoint issue

Un endpoint `issue` se hashea como el contenido del archivo del ítem, y sus estados son los de un endpoint estructural sin capture: `PENDING`, `OK`, `ALTERED`, `UNRESOLVED` cuando el ítem no se encuentra.

## Escritura de cache

### `check` escribe en un solo archivo, y nada más

`.bilink/cache/state`:

- `range`: byte range absoluto del fragmento, cuando la resolución lo encuentra.
- `state`: estado de resolución del capture.
- `state.N`: estado de aceptación, por endpoint.

Ni el bilink ni el capture se tocan. Verificar no produce un diff.

Y `check` no propaga. Refrescar la cache no cambia ningún valor aceptado, así que el vecino de la cadena no ve nada. La cadena la mueve `accept`, que es quien escribe una decisión.

Con cache fría el estado no está disponible y hay que correr `check`. Es distinto de `commit`, que con cache fría sólo cuesta más ([cache.md](cache.md)).

## Fuente del cambio

### Para endpoints estructurales no-OK, se reporta el origen

| Condición git | Fuente en salida |
|---|---|
| `git diff -- <file>` tiene hunks solapando el fragmento | `[UNSTAGED]` |
| `git diff --cached -- <file>` tiene hunks solapando el fragmento | `[STAGED]` |
| `git log <commit>..HEAD -- <file>` tiene commits | `[commit <hash> "<msg>"]` |

El baseline es `commit` —el commit en que el fragmento quedó con el contenido aceptado— y no un timestamp: `git log` recorre por ancestría y es exacto, mientras que las fechas se desordenan con rebases y cherry-picks. `commit` vive en [la cache](cache.md) y con cache fría se re-deriva. Como esta atribución sólo corre sobre endpoints ya no-OK, el costo está acotado por lo que está roto.

### Intersección hunk / fragmento

```
fragmento: líneas F_start–F_end  (derivadas del range, en bytes)
hunk:      @@ -H_start,H_count +...

H_start + H_count < F_start  → BEFORE  (el fragmento se corrió, no cambió)
H_start > F_end              → AFTER   (irrelevante)
se superpone                 → WITHIN  (causa de EXPANDED, ALTERED, REANCHORED)
```

Un `BEFORE` no produce ningún estado: el fragmento es un nodo entero y la query lo encuentra corrido sin ayuda. Lo que el corrimiento sí puede alimentar es a lattice (decisión `displacement-por-hunks`).

## Fix disponible

### Sólo los estados del capture tienen fix, y ninguno cierra solo

`MOVED` y `REANCHORED` los repunta [`apply`](apply.md), que no lee el estado cacheado: re-resuelve el capture contra el árbol actual. Nunca se aplican solos.

Y ninguno cierra solo: `apply` repunta y deja el endpoint en `RELOCATED`, que sale con 1 hasta que alguien acepte.

Los dos son estados del capture —dónde está el fragmento—. Ningún estado de aceptación tiene fix automático: aprobar un contenido es una decisión.

## Salida

### Qué se imprime y qué código de salida se devuelve son dos preguntas distintas

Los endpoints en `OK` se omiten por defecto. Con `--verbose` se muestran todos.

Un endpoint que no está `OK` se imprime, siempre, porque hay trabajo que hacer. Cuál de esos trabajos hace fallar a `check` es otra cosa. Confundir las dos deja estados que existen en disco y no aparecen en ninguna parte.

```
$ bilinker check .

7f3d8e9a  (OK, CHAIN_DIRTY)
  endpoint.1  → path >impl   el vecino fue re-aceptado
  → inspeccionar: bilinker chain status 7f3d8e9a-…

3a4b5c6d  (RELOCATED, ALTERED)
  endpoint.0  la ubicación cambió y nadie la aprobó
    aceptado: capture 67ba7217…  specs/voting.yaml
    ahora:    capture 9f8e7d6c…  specs/domain/voting.yaml
  → revisar y aprobar: bilinker accept 3a4b5c6d.0 --place
  endpoint.1  java-demo::Persona#vote  el AST interno cambió
    - Comparator.comparingInt(String::length)
    + (a, b) -> a.length() - b.length()
    source: commit c7d3e9f "Inline comparator" (2026-05-19)

f1e2d3c4  (EXPANDED, OK)
  endpoint.0  specs/reporter.yaml#generate  el fragmento creció — AST sin cambios
    + log.info("called");  [commit a3f2b1c "Add audit log"]
  → fix disponible: bilinker apply
```

`OK_N1_UNCONFIRMED` tampoco se imprime por endpoint: con `--no-ask-n1` sale en cada endpoint con nivel 1, y una línea por cada uno taparía el trabajo. Se cuenta al final:

```
12 endpoint(s) OK_N1_UNCONFIRMED: el nivel 1 no se confirmó (--no-ask-n1).
  Confirmarlos:  lspd start --wait --lang rust && bilinker check .
  Listarlos:     bilinker status | grep OK_N1_UNCONFIRMED
```

### Código de salida de `check`

| Código | Condición |
|---|---|
| 0 | Todos los captures resuelven y todos los endpoints están en `OK`, `EXPANDED`, `RESTYLED` u `OK_N1_UNCONFIRMED`. |
| 1 | Algún capture en `UNANCHORED`, `DELETED` o `BROKEN`, o algún endpoint en `RELOCATED`, `ALTERED`, `UNRESOLVED`, `PENDING`, `CHAIN_DIRTY`, `CONSENSUS_DIVERGED`, `CONTRACT_RESTYLED`, `CONTRACT_ALTERED`, `CONTRACT_RELOCATED` o `CONTRACT_UNLOCATED`. |
| 1 | Algún bilink no se pudo leer, aunque todos los que se leyeron estén `OK`. |
| 2 | La versión de formato de la capa no se entiende. No se verificó nada. |
| 2 | Hay nivel 1 adquirido y el daemon no contesta, o falla, o Ctrl-C cortó su espera. No se terminó de verificar. |

Sin flag, un `check` que sale con 0 confirmó todo nivel 1. No hay un estado para *"pregunté y no pude"*: un daemon que indexa se espera, y uno que falla es un `check` que no terminó.

`OK_N1_UNCONFIRMED` sale con 0 porque se pidió no preguntar: lo que git y tree-sitter pueden mirar está bien, y el resumen dice cuántos quedaron sin confirmar.

`CONTRACT_UNLOCATED` sale con 1: es trabajo escrito en el archivo, puesto ahí por algo que ya pasó, y sale con 1 por lo mismo que `PENDING`.

`RELOCATED` sale con 1. Repuntar no aprueba, y un vínculo apuntando a un fragmento que nadie miró es trabajo pendiente, no un detalle de mantenimiento.

Un bilink ilegible sale con 1 por lo mismo que `PENDING`: hay trabajo que hacer y nadie lo hizo.

Y la versión sale con 2, no con 1, porque no es lo mismo *"hay endpoints no-OK"* que *"no leí nada"*. Un CI que trata cualquier no-cero igual no nota la diferencia, y uno que sí puede distinguir entre un drift que hay que revisar y una capa que hay que migrar.

## `bilinker status`

### Muestra la cache agrupada por archivo, sin re-verificar

```
bilinker status [<path>]
```

| Argumento | Descripción |
|-----------|-------------|
| `<path>` | Directorio de la capa a inspeccionar. Por defecto: directorio actual. |

```
$ bilinker status

commands/
  accept.md   f715f67e  (OK, OK)
              6f9fa32c  (OK, OK)
  graph.md    49218eae  (OK, OK)
              298e1b92  (OK, OK)

concepts/
  bilink.md   1e318d3a  (OK, OK)
```

Cada línea muestra el nombre del archivo (solo en la primera aparición), el UUID corto (8 chars) y el estado de ambos endpoints: `(state.0, state.1)`, cada uno con sus [dimensiones](#la-dimensión-califica-al-estado-y-no-lo-reemplaza) al lado cuando las tiene. Los bilinks se agrupan por el directorio del endpoint estructural. Un bilink sin endpoint estructural —los dos son `path`— aparece bajo `(layer)`.

Es sólo lectura: no modifica ningún archivo, ni siquiera la cache, y no re-ejecuta queries. Sale siempre con 0.

### Con la cache fría no hay nada que mostrar

`status` lee [`cache/state`](cache.md); no resuelve ninguna query. Y la cache no está en git, así que un clon fresco no tiene estados:

```
$ bilinker status

sin estados: la cache está fría.
  Correr `bilinker check .` para calcularlos.
```

No es un error: la cache fría es un estado normal y `check` es offline y barato. Lo que `status` no hace es inventar: mostrar `OK` sin haber verificado sería peor que no mostrar nada.

Si la cache corresponde a otra rama, se descarta sola y el resultado es el mismo.

## `bilinker watch`

### Reporta en tiempo real los archivos vinculados que se modifican

```
bilinker watch
```

Sin argumentos. Opera sobre la capa actual.

1. Inicia un watcher sobre el directorio raíz de la capa.
2. Cuando un archivo modificado está referenciado por al menos un bilink estructural, emite una línea por cada cadena afectada.
3. Continúa hasta recibir Ctrl-C.

Los archivos dentro de `.bilink/` se ignoran.

```
$ bilinker watch
watching /home/user/proyecto/subsystems/stratum  (Ctrl-C to stop)

ALTERED  crates/stratum-cli/src/main.rs  chain 8e2e749a-fbb9-44aa-9b7f-a57972498371
ALTERED  crates/stratum/src/path.rs      chain 9cfe0db7-65d3-4b26-bb08-ba661dcb071d
```

Cada línea lleva `ALTERED` —el estado esperado al correr `bilinker check`—, el path relativo del archivo modificado y el UUID completo de la cadena afectada.

`watch` no actualiza los archivos `.bilink` ni cambia estados: sólo notifica. Para confirmar el drift y actualizar el estado, correr `bilinker check` y luego `bilinker accept`. El watcher usa eventos del SO (inotify en Linux, FSEvents en macOS).

| Código | Condición |
|--------|-----------|
| 0 | Terminado por Ctrl-C. |
| 1 | Error al iniciar el watcher. |
