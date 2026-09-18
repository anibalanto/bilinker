# El bilink

Un bilink es una declaración y dos decisiones: qué dos cosas están vinculadas, y qué versión de cada una alguien aprobó. Nada más entra al archivo. Todo lo demás se puede reconstruir y vive en [la cache](cache.md).

## El archivo

### El bilink vive en `.bilink/<uuid>.yaml`, y el UUID es el id de la cadena

Los bilinks viven en carpetas `.bilink/` dentro de cada capa del proyecto. El nombre del archivo es un UUID v4: es a la vez el identificador de la cadena y el mecanismo de localización entre capas.

```
proyecto/
  .bilink/
    7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml   ← tip (capa spec)
  .stratum/
    impl/
      .bilink/
        7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml   ← tip (capa impl)
```

El mismo UUID aparece en todas las capas que participan de una cadena.

La extensión es `.yaml`. El tipo lo dice la carpeta que lo contiene; repetirlo en el nombre sería redundante.

Esas carpetas están en el árbol de trabajo y no en ninguna rama del proyecto: viven en `refs/bilink/<branch>`, una ref por rama, y el árbol las lleva materializadas y excluidas del índice del proyecto ([ref.md](ref.md)).

### La estructura del archivo: dos endpoints, y en cada uno `link`, `n` y `accepted`

```yaml
endpoint:
  0:
    link: capture 67ba7217e0334051becd4921b55a7872
    n:
      1:
        link: capture fe74f8b4e9fd72eeae03ea41ce520155 1b06e7c6750d68696653c9112925a54e
    accepted:
    - agree:
      - pablo
      link: capture 67ba7217e0334051becd4921b55a7872
      hash: c00e07602bd560755096b57df1ddb9ed49d816fb8af58a4ec9cde82f21f38db3
      hash_ast: 1b9e44a2f0c8d3e7a5b1c9d4e2f6a8b0c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3
      n:
        1:
          link: capture fe74f8b4e9fd72eeae03ea41ce520155 1b06e7c6750d68696653c9112925a54e
          hash: ebdaf622a00a28d0d45d27a793ebe10a8c8c14637259fe11e4f5b82aa739b6b7
          hash_ast: 49b10d85fc2f5a7a6ecb55108007419f31acf617cceb62601fd8b14890d7b856
  1:
    link: path >impl
```

Dos ejes por endpoint, y cada uno con su declaración y su decisión. El fragmento y su [vecindario](accept.md) se escriben con la misma forma, y cada campo tiene un escritor y uno solo:

| | declaración | decisión |
|---|---|---|
| el fragmento | `link` | `accepted[].link` |
| su vecindario | `n.1.link` | `accepted[].n.1.link` |
| sus partes | `dimensions.<nombre>.query` | `accepted[].dimensions.<nombre>` |

Las [dimensiones](#las-dimensiones-parten-el-contenido-del-fragmento) no son un eje más: parten el contenido del fragmento.

`apply` escribe las declaraciones de `link` y de `n`, y el generador que capturó el extremo, la de `dimensions`. `accept` escribe las decisiones. `check` no escribe nada en el bilink. La frontera no es una convención de nombres: es estructura.

`accepted` es una lista, porque dos personas pueden haber aprobado versiones distintas del mismo fragmento y ninguna de las dos se descarta. Más de una entrada es un estado, `CONSENSUS_DIVERGED`, y no un modo de operación.

No existe campo `id`: el UUID del nombre es el identificador. No existe `range`: la ubicación vive en el [capture](capture.md) que el `link` referencia. No existe `resolved_at`, ni `state`, ni `commit`: son derivados y viven en [la cache](cache.md).

### El `link` de un nivel del vecindario, y su tercera forma

El `link` de un nivel es el eje de su ubicación, y toma tres formas:

| | |
|---|---|
| *(ausente)* | se miró y no hay vecinos: una firma cuyos tipos son todos de otra capa |
| `capture <id> <id> …` | éstos son los vecinos, ordenados por id |
| `unknown` | el contrato está y de qué vecinos salió no se sabe |

`unknown` no es un vecindario vacío ni una renuncia: el nivel está adquirido —`hash` y `hash_ast` siguen ahí, y el eje del contenido se verifica igual— y lo único que falta es la ubicación. Aparece cuando alguien tuvo los hashes sin poder resolver los captures: una migración que no pudo traerlos, o un consumidor que recibe los del proveedor sin poder resolver captures ajenos.

Va en el `link` porque es el estado de un eje de un nivel. Un `n: unknown` al lado de `n: declined` sería un estado del vecindario entero, y escribirlo tiraría los hashes, que son justo la parte que sí se tiene. `unknown` deja el otro eje en pie: es la misma partición que ya gobierna el fragmento.

Y es un valor del slot, no un campo hermano. Un `unknown: true` al lado de `link` deja escribible `link: capture <id>` con `unknown: true` encima, que no quiere decir nada: la misma familia de combinación inválida por la que `n` es un campo con tres estados y no tres campos sueltos. En el mismo slot la contradicción no se puede escribir.

Vale en los dos lados —`n.1.link` y `accepted[].n.1.link`— porque ninguno de los dos se puede inventar: `apply` mantiene la declaración y tampoco sabe de dónde salió un contrato restituido.

Que un `link` ausente ya parsee no lo habilita como sustituto. La ausencia ya significa *"se miró y no hay vecinos"*, y darle un segundo significado es el mismo error que `n: declined` puesto donde iba una imposibilidad, un nivel más adentro.

Dos `unknown` no son la misma ubicación. El eje se decide comparando dos ids y acá no hay ids de ninguno de los dos lados: la comparación no se puede hacer, y no hacerla no es que coincida. Ese eje no queda limpio, y cómo lo nombra `check` va con su reporte ([check.md](check.md)), no con el formato.

No comparte grilla con los prefijos de un endpoint. Los cinco de "Tipos de endpoint" contestan *dónde*, y ahí `unknown` no entra: un endpoint sin ubicación conocida no tiene contenido aprobado que proteger, así que su forma de decirlo es no tener `accepted`.

### Las dimensiones parten el contenido del fragmento

`dimensions` dice qué partes del fragmento se vigilan, cada una con un nombre. Va en el endpoint y no en el [capture](capture.md): qué se vigila de un fragmento es una decisión, y el capture es una ubicación y nada más. Dos bilinks sobre el mismo capture pueden vigilar partes distintas.

```yaml
endpoint:
  0:
    link: capture 67ba7217e0334051becd4921b55a7872
    dimensions:
      parameters:
        query: |-
          (method_declaration parameters: (formal_parameters) @target)
      return:
        query: |-
          (method_declaration type: (_) @target)
    accepted:
    - agree:
      - pablo
      link: capture 67ba7217e0334051becd4921b55a7872
      hash: c00e07602bd560755096b57df1ddb9ed49d816fb8af58a4ec9cde82f21f38db3
      hash_ast: 1b9e44a2f0c8d3e7a5b1c9d4e2f6a8b0c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3
      dimensions:
        parameters:
          hash: 5d0c8a13e7f2b94c61a0d38e25f7b1c9a4e6d0f3b8c2a57e19d4f6b0c3a8e2d7
          hash_ast: 9a31f0c7b2e84d15a6c9e3f07b1d2a48c5e9f6b3d0a7c1e48f2b5d9a6c3e0f17
        return:
          hash: e27b4f91c0a3d68e25b7f1c4a9d03e6b8f2c5a17d9e4b0f3c6a8e1d27b5f9c04
          hash_ast: 3c7e0a9d5f1b84e26c0d9a7f3e5b1c48d2a6f0e9b7c3d15a8e4f2b6c0d9a7e31
    as: spring-controller
```

La declaración lleva, por nombre, la `query` que encuentra la parte: una query tree-sitter de la gramática del archivo, con sus `@target`. La decisión lleva, por nombre, el `hash` y el `hash_ast` de lo que se aprobó de esa parte, con la forma de un nivel del vecindario: `hash` es el SHA-256 del texto de la parte y es obligatorio, y `hash_ast` va sólo donde el AST discrimina contenido. Un `hash_ast` sin su `hash` no es una dimensión, y se rechaza.

La query va escrita en el endpoint y no se deduce del [`as`](#as): resolver una dimensión no pide el generador instalado, igual que un `as` que nombra uno ausente no es un error.

Ausente y vacío son lo mismo, en la declaración y en la decisión: el endpoint no declara partes. Las claves se escriben ordenadas por nombre.

### El nombre de una dimensión es una etiqueta opaca

Los nombres son del generador —`route`, `parameters`, `body`—, y el formato no tiene una tabla de ellos: no hay un conjunto de partes que valga para toda gramática. En un método de Java las excepciones son una parte suelta, en una función de Rust van adentro del retorno, y en TypeScript no existen.

bilinker los compara y no los interpreta. Una dimensión de la declaración se empareja con la de la decisión que lleva su mismo nombre, y el nombre no dice nada más. Por eso **un nombre desconocido no es un error**: no hay lista contra la cual desconocerlo. No contradice que los campos desconocidos se rechacen, porque el nombre no es un campo: es una clave que el archivo elige, como el uuid de un bilink.

Es lo que deja a las dimensiones cruzar la frontera: un consumidor que no tiene el plugin del generador compara nombres y hashes igual que el proveedor.

### Una dimensión resuelve relativa al nodo del capture, y nunca ancla por su cuenta

La query de una dimensión no se evalúa sobre el archivo: se evalúa desde el nodo que el capture fijó. Puede nombrar partes de ese nodo y de sus ancestros —*"el `@RequestMapping` de la clase que contiene a este método"*—, porque no busca de qué método se trata: eso ya lo dijo el capture.

Por eso ninguna dimensión ancla. Si el capture no resuelve, no hay nodo desde el cual evaluarlas, y ninguna resuelve. Y la dimensión no depende de nada adentro de la query del capture, ni de sus nombres de captura ni de su forma: sólo del nodo que esa query identifica.

### Una dimensión de varios `@target` es su concatenación, en orden de archivo

La query de una dimensión puede llevar más de un `@target`: la ruta de un endpoint sale de dos anotaciones —`@RequestMapping` en la clase, `@GetMapping` en el método—, que como texto completo no existe en ningún nodo. El texto de la dimensión es la concatenación de sus rangos en el orden en que aparecen en el archivo, no en el que la query los nombra, que es un detalle de cómo se escribió el patrón y no del documento.

Cada rango se recorta en sus bordes por separado, antes de concatenar, igual que el del [capture](capture.md). Recortar la concatenación dejaría los bordes internos a merced de dónde termina un nodo y empieza el otro, que es justo el contexto del que el recorte existe para independizar.

### Las partes de una dimensión se unen con `\n`

Los rangos no son contiguos, así que hay que decidir qué va entre uno y el siguiente, y eso entra en el `hash`:

| | |
|---|---|
| nada | dos capturas pegadas producen un texto que no existe en ningún archivo |
| `\n` | elegido: legible, y estable frente a cuánto espacio haya en el medio |
| el texto intermedio | es el archivo tal cual, pero entonces lo que queda en el medio —el cuerpo, entre dos anotaciones— entra por la ventana |

Y no se vuelve a tocar. Cambiarlo movería de una vez el hash de todas las partes compuestas, y pasarían a `ALTERED` sin que nadie tocara el código.

`hash_ast` sigue la misma regla: las s-expressions de los nodos, en el mismo orden, unidas por `\n`.

### Ninguna parte de una dimensión contiene a otra

El texto de una dimensión es la concatenación de sus partes, así que una parte adentro de otra se contaría dos veces y el hash pasaría a depender de un solapamiento que nadie quiso. Los rangos de una dimensión son disjuntos.

Dos partes sí pueden compartir una línea, y es el caso normal: en `public Dto get(String t)` el tipo de retorno y los parámetros son rangos disjuntos de la misma línea, con el nombre del método en el medio y afuera. Mostrarlos no es imprimirlos uno detrás del otro, porque esa línea saldría repetida: es problema de quien lo muestra, y lo resuelve [`get`](get.md). El `hash` no cambia.

### Más de un `accepted` es un estado, no una forma de trabajar

Un endpoint sólo puede estar `OK` con exactamente una entrada. Con dos o más el estado es `CONSENSUS_DIVERGED`, y `check` falla.

Eso es lo que vuelve sana a la lista: no es una estructura para sostener dos verdades, es una forma de no perder ninguna mientras se resuelve. Es transitoria por construcción: alguien mira, acepta, y colapsa a una.

La alternativa —*"una gobierna y las demás son historia"*— daría un endpoint verde con una aprobación vieja adentro, que es la clase de mentira que el resto del formato existe para impedir.

Es un eje que no describe al fragmento. Los demás estados dicen dónde está, qué dice y qué tipos menciona. Éste dice *"sobre esos tres no hay una sola respuesta"*, y de qué lado está el desacuerdo es de las personas, no del código.

### Cómo colapsa, que es la regla que ya existía

`accept` sobre un endpoint divergido deja una sola entrada: la de los valores que se están aprobando. Las entradas cuyos valores difieren se van.

Es exactamente lo que [`agree`](accept.md) ya hacía: quien aprobó el hash anterior no aprobó el nuevo, y los aprobadores anteriores quedan donde siempre estuvieron, en los commits que escribieron los valores anteriores. Lo único que la lista cambia es qué pasa entre las dos aceptaciones: conviven visibles hasta que alguien resuelve.

Y si los valores que se aceptan coinciden con los de una entrada existente, no hay colapso que hacer: quien acepta se suma a su `agree`, y las demás entradas siguen ahí. Sigue divergido, y es correcto: sumarse a un lado no resuelve un desacuerdo.

### Una entrada es completa, y por eso lleva un solo `agree`

Si una entrada está escrita, se aceptó entera. No hay endoso parcial de una entrada, y por eso un `agree` adentro de `n.1` no nombra nada.

Y no hay cómo aceptar a medias. Con firma resoluble y sin nivel 1 que conservar, `accept` sin daemon se niega: no tenés el mapa completo del vecindario, así que no hay nada que aprobar. La única alternativa es declararlo con `--decline-n1`, que es renunciar y no abstenerse. No existe el camino *"apruebo la firma y el vecindario no lo miré"*.

Lo que sí existe es que dos personas aprueben el mismo fragmento y vecindarios distintos, y eso ya tiene forma: son dos entradas.

```yaml
accepted:
- agree:
  - anibal
  link: capture <a>
  hash: h1
  n: { 1: { link: capture <n-a>, hash: hA } }

- agree:
  - juan                   # mismo fragmento que Pedro…
  link: capture <b>
  hash: h2
  n: { 1: { link: capture <n-b>, hash: hB } }

- agree:
  - pedro                  # …y otro vecindario
  link: capture <b>
  hash: h2
  n: { 1: { link: capture <n-c>, hash: hC } }
```

`agree` va en bloque incluso acá, donde hay un nombre por entrada. Compactarlo a `agree: [anibal]` le saca al campo lo que lo hace servir: `git blame` sólo atribuye una línea a un commit, así que el día que la entrada tenga dos firmantes en una línea el primero se pierde. Que en `n` de acá arriba sí esté compactado no es una excepción: ahí la forma es tipografía, y en `agree` es el mecanismo.

Juan y Pedro coinciden en `link` y `hash` y difieren en `n`. Son dos contratos distintos, y por lo tanto dos entradas, no una entrada con dos endosos parciales. La identidad de una entrada es su tupla entera: dos personas convergen en una sola entrada sólo si coinciden en todo, y si difieren en cualquier nivel, difieren.

### Lo que no está resuelto: si cruza la frontera

Un endpoint `repo` copia el `accepted` del proveedor. Con el proveedor divergido no hay uno solo que copiar, y las tres salidas son malas de distinta forma:

| | |
|---|---|
| no copiar nada | el consumidor queda bloqueado por un desacuerdo interno del proveedor, del que no es parte |
| copiar la lista | el consumidor hereda un desacuerdo ajeno y su propio `accepted` deja de ser una decisión suya |
| rechazar y decirlo | honesto, pero pide un estado propio del lado del consumidor, porque el consumidor no está divergido |

Queda abierta a propósito: es un caso que no se puede alcanzar hasta que haya un proveedor real con divergencia, y decidirlo antes sería inventar el nombre de un estado que nadie vio.

## Tipos de endpoint

### El tipo es explícito, en un prefijo

Un `link` lleva el tipo adelante, en un prefijo. El resto del valor se interpreta en el lenguaje que el prefijo nombra, que es lo que el parser necesita saber:

```
link: <prefijo> <resto>
```

Partir en el primer espacio y matchear el prefijo. Nada más.

| Prefijo | El resto es | Estado |
|---|---|---|
| `capture <id>` | un id de [capture](capture.md) de esta capa | implementado |
| `path <stratum-path>` | un path Stratum hacia una capa vecina | implementado |
| `issue <id>` | un id de ítem del tracker | implementado |
| `repo <alias>` | un alias de repo ajeno, declarado en `.bilink/.{alias}.toml` | implementado |
| `abstract` | nada: la punta abierta de un bilink que otro proyecto consume | implementado |
| `bilink <uuid>` | otro bilink | decisión `bilink-endpoint`, sin implementar |

`path` y no `layer` porque un stratum-path también cruza a sub-proyectos —`*/subsystems/lattice`— que el modelo de capas de stratum distingue de las capas internas: `layer` afirmaría de más.

Los dos últimos implementados son [la frontera entre proyectos](frontier.md), y son aditivos: ningún archivo existente los usa, y todos siguen siendo válidos.

Un endpoint estructural no describe el fragmento: referencia un capture. Lo que sí es propio de cada endpoint es qué se aceptó: dos bilinks sobre el mismo capture pueden haber aprobado contenidos distintos y reportar estados distintos.

### Un prefijo desconocido es un error, no un fallback

Sin fallback no hay desempate. Con un endpoint de capa que fuera lo que queda cuando ninguna otra forma matchea, haría falta una regla de precedencia entre prefijos, palabras reservadas y paths. Con el tipo adelante, esa regla no hace falta.

Los prefijos reconocidos se publican en el esquema, y salen de la misma tabla que usa el parser. Agregar un tipo obliga a tocarla, y eso cambia el hash del esquema ([format-version.md](format-version.md)).

### Endpoint capture

Identifica un fragmento, referenciando el [capture](capture.md) que lo ubica.

```
link: capture 67ba7217e0334051becd4921b55a7872
```

El id es el hash de la ubicación. El endpoint no describe el fragmento: pregunta.

```yaml
accepted:
- link: capture 67ba7217e0334051becd4921b55a7872   # la ubicación aprobada
  hash: c00e07602bd5…                              # el contenido aprobado
  hash_ast: 1b9e44a2f0c8…                          # opcional
```

Las dos dimensiones se aprueban por separado ([accept.md](accept.md)).

### Endpoint path

Identifica una capa vecina con un path Stratum.

```
link: path >impl
link: path <
```

La ruta es relativa a la raíz de la capa actual. `.bilink/` es implícito: nunca aparece en el valor.

### Resolución de un endpoint `path`

Dado `link: path <stratum-path>` en `<capa-actual>/.bilink/<uuid>.yaml`:

1. Resolver el path Stratum tomando como base la raíz de la capa actual.
2. Usar el resultado como `<layer-path>`:

```
resolved = ../<layer-path>/.bilink/<uuid>.yaml
```

El `../` sube del directorio `.bilink/` a la raíz de la capa. La carpeta `.bilink/` nunca aparece en el valor.

Su `accepted` es una copia de `accepted.link` y `accepted.hash` del endpoint estructural del bilink adyacente. No es el hash del archivo vecino completo. Cada valor cambia por una sola razón: `hash` cuando cambia el contenido publicado, `link` cuando cambia su ubicación aprobada. Los dos son inmunes a etiquetas, comentarios y reordenamientos del archivo vecino.

Los endpoints `path` no tienen capture: apuntan a una capa, no a un fragmento.

### Endpoint issue

Identifica un ítem del tracker: una épica, una user story o una task.

```
link: issue 3a
```

Resuelve contra el panorama del worklist, nunca contra una ventana:

```
<project-root>/.worklist/insecure/all/<id>.<tipo>.md
```

El project root se encuentra subiendo `depth * 2` componentes desde la capa actual.

`.worklist/` no es un directorio de ítems: es un contenedor de worktrees, y `insecure/all` es el único que los tiene todos. Una ventana —`secure/sprint/<id>`— lleva el subárbol de su sprint y nada más, así que un endpoint que resolviera contra la que está abierta fallaría según en qué rama esté el checkout: el mismo `issue 3a` resolvería o no según el sprint del momento, y un bilink válido pasaría a no-OK sin que nadie toque nada.

El tipo no está en el endpoint y no hace falta que esté. Los ítems son archivos sueltos en un solo directorio y sus ids son únicos, así que el archivo se encuentra por prefijo. Si no matchea nada el endpoint no resuelve; si matchea más de uno, el worklist tiene dos ítems con el mismo id y eso es un error suyo. Que el tipo quede afuera es lo que hace que el endpoint sobreviva a la planificación: recolgar un ítem de otra user story cambia un campo del ítem, no el nombre de su archivo.

Se llama `issue` y no `task` porque apunta a cualquiera de los tres tipos, y `task` es además el nombre del tipo hoja del worklist. El nombre sale de qué es la cosa del otro extremo, no de quién la provee.

Su `accepted` lleva `hash` y no lleva `link`: no hay capture que aprobar, porque la ubicación de un ítem es su id.

El worklist está deprecado y lo reemplaza `muckpile`; a dónde resuelve `issue <id>` con `muckpile` es de la decisión `decisiones-vivas`, en accreta.

### Endpoint `abstract`

Una punta abierta: no la resuelve quien la declara, la aporta quien la consuma.

```
link: abstract
```

No lleva valor y no lleva `accepted`. No hay nada que bendecir del lado abierto, y con el bloque entero ausente eso es una ausencia y no una lista de campos vacíos.

Es palabra reservada, y con el tipo adelante no hace falta ninguna regla de desempate: es la única forma sin valor, y ninguna otra se le parece.

Su estado es `OPEN`, constante. Siempre sana, nunca pide acción, y `accept .` nunca la toca ([frontier.md](frontier.md)).

### Endpoint repo

Identifica un fragmento de otro proyecto, por un alias local.

```
link: repo hsi
```

El UUID es el mismo que el del bilink remoto, así que no se escribe. El alias se declara en `.bilink/.{alias}.toml`, y es el único lugar del consumidor que sabe algo del otro repo: el `link` no contiene ninguna URL.

```
resolved = <clon de .{alias}.toml @ refs/bilink/{branch}>/.bilink/<uuid>.yaml
```

Es el endpoint `path` generalizado: misma convención de UUID compartido, mismo `.bilink/` implícito; sólo cambia que la dirección se resuelve por alias en vez de por path relativo. El `.toml` declara la rama del proyecto; la traducción a `refs/bilink/<branch>` la hace la herramienta.

Su `accepted` es una copia de `accepted.link` y `accepted.hash` del endpoint estructural del bilink remoto. Dos SHA-256 opacos: ninguno revela path, query, texto ni commit del proveedor.

```yaml
accepted:
- link: capture 8f2a4c6e…   # el capture del proveedor — ubicación publicada
  hash: c4e1770b…           # hash del fragmento del proveedor — contenido publicado
```

Es una copia opaca: se compara, no se resuelve. El `capture <id>` que lleva adentro es de la capa del proveedor, no de ésta, y buscarlo acá no encontraría nada. Eso ya vale para un endpoint `path`, donde el id copiado es del vecino. La forma es la misma en los dos casos, y por eso el campo se lee igual.

## Topología

### Link directo (misma capa)

Un bilink puede conectar dos fragmentos dentro de la misma capa: los dos `link` son endpoints estructurales. Hay un único archivo, no hay traversal.

```
[fragmento A] ←→ [fragmento B]
```

### Cadena entre capas

Una cadena es una secuencia lineal de bilinks con el mismo UUID que conecta dos fragmentos a través de una o más capas ([chain.md](chain.md)):

- tip: un endpoint estructural más un endpoint `path`. Son los extremos, y siempre hay exactamente dos.
- mid: los dos endpoints son `path`. Puede haber cero o más.

```
[fragmento] ←→ tip ←→ mid* ←→ tip ←→ [fragmento]
```

La topología es estrictamente lineal: sin ciclos ni bifurcaciones.

### Bilink con capa no creada todavía

Un endpoint `path` puede apuntar a una capa que aún no existe. El estado `TODO` dice que la conexión está planeada, no que haya un error. Una vez creada la capa y aceptado el endpoint, pasa a `OK`.

## Campos semánticos

### `kind`, `name` y `as` son inertes

Opcionales, y no afectan ningún hash ni ningún estado. Son declaración, así que van al lado de `link` y los escribe quien escribe `link`.

```yaml
kind: governs
endpoint:
  0:
    link: capture 67ba7217e0334051becd4921b55a7872
    name: architecture-decision
  1:
    link: path >impl
    name: spec-impl-bridge
```

### `kind`

Clasifica la relación. Valor libre; valores definidos:

| Valor | Significado |
|-------|-------------|
| *(ausente)* | Vínculo estructural: relación de implementación entre fragmentos |
| `governs` | Decisión o documento que gobierna un vínculo entre capas |

`governs` es el único valor definido y todavía no se puede expresar: exige que un `link` apunte a otro bilink, y ese tipo de endpoint es de la decisión `bilink-endpoint`. El campo existe y se preserva; su valor documentado espera a que llegue el endpoint.

No existe un `kind` para relaciones de llamada. Un bilink declara una referencia que un humano aceptó; las aristas de llamada las deriva una herramienta del código actual y las agrega lattice como aristas `derived`. Declararlas a mano crearía un duplicado permanente que hay que mantener sincronizado.

### `name`

Etiqueta del rol semántico del endpoint en la relación que `kind` declara. Texto libre. Va adentro del endpoint, no como `name.N` suelto: es un dato de una punta y ahí hay dónde ponerlo.

### `as`

Con qué generador se capturó ese extremo. El valor es el mismo nombre que tomó [`--as.N`](chain.md) al escribirlo —`interface`, `spring-controller`—, y su ausencia significa que no se sabe con qué se capturó: es lo que dice cualquier archivo escrito antes de este campo, y lo que dice un capture del núcleo.

```yaml
endpoint:
  0:
    link: capture 67ba7217e0334051becd4921b55a7872
    as: spring-controller
  1:
    link: abstract
```

Va por endpoint porque el hecho es de un extremo. Un tip puede ser un endpoint de Spring y el otro `abstract`; un campo arriba, al lado de `kind`, afirmaría sobre la relación entera algo que vale de un lado solo.

Y no entra en `kind`, que ya contesta otra pregunta: `kind` clasifica qué clase de relación se declara y `as` dice con qué receta se capturó este extremo.

Es la receta, no el valor. Lo que se guarda no es el nombre ni la ruta que el generador sabe componer —eso sale del fragmento cada vez que se lee, que es lo que no puede mentir— sino con qué componerlos. Un valor derivado y guardado envejece en silencio; la regla que lo deriva no cambia cuando cambia el valor.

Un `as` que nombra un generador que no está instalado es un dato que no se pudo usar, nunca un error. El capture sigue resolviendo y `check` sigue contestando: lo único que se degrada es lo que ese generador sabía componer.

## Estados

Ningún estado vive en el archivo: `check` los calcula y los escribe en [la cache](cache.md). La grilla completa, con cómo se llega y cómo se sale, es de [check.md](check.md).

### Estados del endpoint estructural

Un endpoint puede desalinearse en dos dimensiones —dónde está y qué dice— y los estados las distinguen. Si el capture no resuelve, eso es estado del capture y el bilink sólo registra que no puede evaluarse.

| Estado | Significado | Cómo se sale |
|--------|-------------|--------------|
| `PENDING` | `accepted` ausente | `bilinker accept` |
| `OK` | La ubicación y el contenido coinciden con lo aceptado | — |
| `RELOCATED` | `link` ≠ `accepted.link`: la ubicación cambió y nadie la aprobó | `bilinker accept --place` |
| `RESTYLED` | El texto difiere pero el AST coincide: sólo formato | `bilinker accept` |
| `ALTERED` | El contenido cambió | revisar + `bilinker accept` |
| `EXPANDED` | El fragmento creció alrededor de lo aceptado | revisar + `accept` |
| `UNRESOLVED` | El capture referenciado no resuelve | `bilinker apply` o `recapture` |
| `CONSENSUS_DIVERGED` | Más de una entrada en `accepted` | `bilinker accept` |
| `CONTRACT_RESTYLED` | El vecindario se reformateó y su AST no cambió | `bilinker accept` |
| `CONTRACT_ALTERED` | Un vecino cambió: el contrato se movió | revisar + `bilinker accept` |
| `CONTRACT_RELOCATED` | El conjunto de vecinos declarado ≠ el aceptado | revisar + `bilinker accept` |
| `CONTRACT_UNLOCATED` | El contrato está y su ubicación es `unknown` | acuñar sus captures + `accept` |
| `OK_N1_UNCONFIRMED` | Todo `OK`, y la resolución de los nombres de la firma no se preguntó: `--no-ask-n1` | `check` con el daemon |

Los `CONTRACT_*` son de un eje aparte: no hablan del fragmento sino de los tipos que su firma menciona ([accept.md](accept.md)). Llevan prefijo por eso: `ALTERED` y `CONTRACT_ALTERED` no son grados de lo mismo, son dos preguntas.

Y sólo aparecen cuando el eje del contenido dice `OK`. Un endpoint tiene un estado y no dos, así que hay que elegir cuál nombrar: si el fragmento mismo cambió, eso se reporta y alguien va a mirar igual. Lo que el eje del vecindario aporta es justamente el caso donde el fragmento no cambió y aun así el contrato se movió.

`UNRESOLVED` absorbe del lado del bilink lo que el capture detalla: el problema no es el vínculo sino la ubicación.

### Estados del endpoint `path`

| Estado | Significado | Cómo se sale |
|--------|-------------|--------------|
| `TODO` | `accepted` ausente y la capa apuntada no existe todavía | Crear la capa + `accept` |
| `PENDING` | `accepted` ausente y la capa existe | `bilinker accept` |
| `OK` | Los dos valores copiados coinciden con los del vecino | — |
| `CHAIN_DIRTY` | El endpoint estructural adyacente fue re-aceptado | `bilinker accept` |
| `LAYER_UNREACHABLE` | La capa está declarada y no clonada | `stratum pull` |
| `LAYER_UNCONFIGURED` | Ni declarada ni presente, con aceptación previa | Declarar la capa · o · `remove` |
| `BROKEN` | La capa existe y el `.bilink` del UUID no está, o el bilink adyacente no tiene endpoint estructural aceptado | Restaurar + `accept` · o · `remove` |

`bilinker remove` elimina el bilink de la capa actual. Los vecinos detectan `BROKEN` en el próximo `check` y deciden: reparar o remover. La remoción se propaga hop a hop.

Las tres ausencias son cosas distintas y se arreglan distinto, que es por qué no comparten nombre: a una capa declarada le falta traerla, a una sin declarar le falta declararla, y un `.bilink` que desapareció bajo una capa presente es una regresión ([frontier.md](frontier.md), "Taxonomía de ausencia").

### Estados del endpoint `abstract`

| Estado | Significado | Cómo se sale |
|--------|-------------|--------------|
| `OPEN` | La punta está abierta a quien la consuma | — |

Constante: no hay contra qué compararla. Nunca pide acción y `accept .` nunca la toca.

### Estados del endpoint repo

| Estado | Significado | Cómo se sale |
|--------|-------------|--------------|
| `PENDING` | `accepted` ausente y el clon está | `bilinker accept` |
| `OK` | Los dos valores copiados coinciden con los del proveedor | — |
| `CHAIN_DIRTY` | El proveedor re-aceptó su fragmento | revisar + `bilinker accept` |
| `REJECTED` | La otra punta dejó de ser `abstract` | investigar: el vínculo no se sostiene |
| `REMOTE_UNREACHABLE` | El repo del proveedor no está clonado | `bilinker fetch <alias>` |
| `BROKEN` | El clon está y el `.bilink` del UUID no | investigar: es regresión |

`CHAIN_DIRTY` no distingue si el proveedor movió el fragmento o cambió su contenido: eso sale de cuál de los dos valores difiere, y lo dice `check`.

## Propagación

### Cada nodo ancla en los valores aceptados del vecino

Un endpoint `path` no ancla en el hash del archivo vecino: guarda una copia del `accepted` del endpoint estructural de ese bilink. La distinción es lo que evita la cascada circular: si hasheara el archivo entero, aceptar un endpoint `path` reescribiría su propio archivo y esa escritura volvería al vecino como un cambio, sin que ningún fragmento se hubiera tocado.

1. El contenido de un fragmento cambia. `check` reporta `ALTERED`.
2. Alguien revisa y acepta: `accepted.hash` del endpoint estructural se actualiza.
3. El nodo adyacente compara su copia contra ese valor → difieren → `CHAIN_DIRTY`.
4. Alguien acepta el endpoint `path` → su copia se sincroniza.

Aceptar un endpoint `path` sólo escribe su propio archivo. Nunca modifica el del vecino, así que no hay cascada circular: la propagación es unidireccional desde el endpoint estructural que cambió hacia los nodos que lo referencian.

Y por eso `check` no propaga nada: refrescar la cache no cambia ningún valor aceptado. Sólo `accept` mueve la cadena.

## Semántica de parseo

### Los campos desconocidos se rechazan

- El archivo es YAML. Los tipos están definidos en Rust y el esquema JSON se genera de ellos ([format-version.md](format-version.md)).
- Los campos desconocidos se rechazan, con el nombre del campo. Descartarlos en silencio es cómo un binario viejo vaciaría las aceptaciones de uno nuevo.
- La aridad es fija: exactamente `0` y `1` bajo `endpoint`. Tres endpoints se rechaza; que falte el `1` también. No es algo que haya que verificar: es algo que no se puede escribir.
- `accepted` sin `hash` se rechaza. Un `hash` suelto fuera del bloque, también.
- Una dimensión aceptada sin `hash` se rechaza, aunque lleve `hash_ast`. Una dimensión declarada sin `query`, también.
- Las claves `0:` y `1:` matchean por nombre, no por posición, y no llevan comillas.
- El archivo usa UTF-8 sin BOM.

## Ejemplo completo: cadena de 2 nodos spec → impl

Cuatro archivos: dos bilinks —uno por capa— y un capture en cada una.

```yaml
# capa spec — .bilink/capture/c1a2b3c4e5f6a7b8c9d0e1f2a3b4c5d6.yaml
file: docs/specs/concepts/check.md
query: |-
  (section (atx_heading (inline) @n0 (#eq? @n0 "Firma"))) @target
```

```yaml
# capa spec — .bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml   (tip)
endpoint:
  0:
    link: capture c1a2b3c4e5f6a7b8c9d0e1f2a3b4c5d6
    accepted:
    - link: capture c1a2b3c4e5f6a7b8c9d0e1f2a3b4c5d6
      hash: a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2
  1:
    link: path >impl
    accepted:
    - link: capture d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0
      hash: b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3
```

```yaml
# capa impl — .bilink/capture/d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0.yaml
file: crates/bilinker/src/check.rs
query: |-
  (function_item name: (identifier) @n0 (#eq? @n0 "check")) @target
```

```yaml
# capa impl — .bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml   (tip)
endpoint:
  0:
    link: path <
    accepted:
    - link: capture c1a2b3c4e5f6a7b8c9d0e1f2a3b4c5d6
      hash: a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2
  1:
    link: capture d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0
    accepted:
    - link: capture d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0
      hash: b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3
```

Cada endpoint `path` copia los dos valores del endpoint estructural de su vecino: qué ubicación y qué contenido se aprobaron ahí.

## Invariantes

1. El nombre del archivo es un UUID v4 válido con extensión `.yaml`.
2. Existen exactamente los endpoints `0` y `1`. La aridad es fija: la multiplicidad la aporta el capture.
3. Un bilink de misma capa tiene dos endpoints estructurales. Una cadena entre capas tiene exactamente dos tips.
4. `accepted` es una lista de cero o más entradas, y cada entrada está completa. La lista vacía y la ausencia son lo mismo: `PENDING`.
5. `accepted.hash` de un endpoint estructural: SHA-256 del fragmento aprobado.
6. Una entrada de `accepted` de un endpoint `path`: copia de `link`, `hash` y `n` de la entrada del endpoint estructural del bilink adyacente. Nunca el hash del archivo vecino. Un vecino divergido no se copia.
7. Un endpoint `issue` se hashea como el contenido del archivo del ítem. No tiene capture, así que su `accepted` no lleva `link`.
8. `state.N = OK` si y sólo si hay exactamente una entrada en `accepted`, y para ella `link` == `accepted[0].link` y el hash actual == `accepted[0].hash`. Con más de una entrada, `state.N = CONSENSUS_DIVERGED`, sin evaluar los otros ejes. El vecindario se compara igual y un nivel más abajo: `n.1.link` contra `accepted[0].n.1.link`, y el fold de hoy contra `accepted[0].n.1.hash`. Sin proveedor ese eje degrada y los otros se deciden igual.
9. El `link` de un endpoint estructural referencia exactamente un capture de su misma capa. Un `n.1.link` referencia cero o más, todos de su misma capa, o es `unknown`, que no referencia ninguno y no es lo mismo que cero.
10. Un bilink no contiene `file` ni `range`: el primero vive en el capture y el segundo en la cache. La única `query` que lleva es la de una dimensión, que no ubica nada. Vale igual para los captures de `n.1.link`.
11. Un bilink no contiene `state`, `commit` ni ningún derivado: viven en la cache.
12. La topología de la cadena es lineal: sin ciclos ni bifurcaciones.
13. Sólo se puede aceptar un endpoint sobre un fragmento commiteado.
14. `kind`, `name` y `as` son inertes: no afectan ningún hash ni ningún estado. `accepted.agree` tampoco los afecta, pero no es decoración: lo escribe `accept` y es parte de la decisión.
15. Un campo desconocido se rechaza con su nombre, nunca se descarta.
16. Ningún `accept` descarta una entrada de `accepted` cuyos valores coincidan con los que se están aprobando: se une el `agree`. Sólo se descartan las entradas que aprobaban otros valores.
17. Un capture referenciado por un `n.1.link` —de la declaración o de una decisión— cuenta como referenciado para `prune`.
18. `unknown` en un `n.N.link` —de la declaración o de una decisión— significa que el nivel está adquirido y su ubicación no se sabe. Es incomparable: dos `unknown` no coinciden, y el eje de la ubicación de ese nivel no queda limpio. El eje del contenido se compara igual, contra el `hash` conservado.
19. El nombre de una dimensión es una etiqueta opaca: se compara, no se interpreta, y un nombre desconocido no es un error. Una dimensión de la declaración se empareja con la de la decisión por su nombre.
20. Una dimensión resuelve desde el nodo que el capture fijó, y nunca ancla por su cuenta: si el capture no resuelve, ninguna dimensión resuelve.
21. Una dimensión aceptada lleva `hash`, y `hash_ast` sólo con él.
