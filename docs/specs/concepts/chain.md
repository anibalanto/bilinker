# Las cadenas

Un bilink conecta exactamente dos fragmentos estructurales. Hay dos formas: un link directo, con los dos endpoints en la misma capa y un solo archivo, y una cadena, donde los fragmentos están en capas distintas y el mismo UUID aparece en un archivo por capa, con endpoints relativos a su posición.

Una cadena es una secuencia lineal de bilinks que conecta dos fragmentos estructurales a través de las capas de un proyecto. Todos los bilinks de una cadena comparten el mismo UUID, que es simultáneamente su identificador de cadena y el nombre de su archivo.

## Topología

### Una cadena tiene dos tips y cero o más mids

```mermaid
flowchart LR
    FA["fragmento A"] <--> TA["tip-A"]
    TA <--> M1["mid₁"]
    M1 <--> M2["mid₂"]
    M2 <--> TB["tip-B"]
    TB <--> FB["fragmento B"]
```

Es el caso mínimo, dos tips y cero mids, cruzando una capa y su impl:

![Cadena de un bilink cruzando dos capas Stratum](chain.svg)

| Tipo de nodo | Endpoint 0 | Endpoint 1 | Posición en cadena |
|---|---|---|---|
| tip | estructural (`capture <id>`) | `path` | extremo (siempre dos por cadena) |
| mid | `path` | `path` | intermedio (cero o más) |

Restricciones de topología: exactamente dos tips por cadena; los tips tienen un endpoint estructural —una referencia a un capture de su propia capa— y uno `path`; los mids tienen ambos endpoints `path`; la cadena es estrictamente lineal, sin ciclos ni bifurcaciones; cada archivo `.bilink/<uuid>.yaml` en una capa pertenece a una sola cadena.

### El UUID identifica la cadena y localiza sus nodos

El UUID v4 es generado una sola vez al crear la cadena (`bilinker chain new`). En cualquier capa, el nodo de la cadena es `.bilink/<uuid>.yaml`.

No existe un archivo de registro central de cadenas: la cadena se descubre recorriendo los endpoints `path` desde cualquier nodo.

### La propagación está integrada en el formato

1. El `accepted` de un endpoint `path` es una copia del `accepted` del endpoint estructural del bilink adyacente —su `link` y su `hash`— y no el hash del archivo vecino.
2. Por eso refrescar la cache no propaga: los estados viven fuera del archivo, y un `accepted` sólo cambia con `accept`.
3. Si alguna de las dos copias ≠ la del nodo adyacente, el próximo `check` detecta `CHAIN_DIRTY`.

```mermaid
flowchart TD
    B(["fragmento B cambia"]) --> TB["tip-B\nstate.0 = ALTERED"]
    TB --> AC(["accept en tip-B\ncambia su hash.0 estructural"])
    AC --> M["mid\nla copia guardada quedó vieja\nstate.1 = CHAIN_DIRTY"]
    M --> TA["tip-A\nstate.1 = CHAIN_DIRTY"]
```

No se requiere índice externo para la propagación: la cadena es autosuficiente.

### El estado de la cadena es el peor de sus nodos

| Estado global | Condición |
|---|---|
| OK | Todos los nodos y fragmentos en estado OK |
| DIRTY | Algún nodo tiene CHAIN_DIRTY (propagación de cambio pendiente) |
| BROKEN | Algún nodo tiene estado terminal (ALTERED, DELETED, UNANCHORED, BROKEN) |

### Ciclo de vida

```
bilinker chain new   → crea los bilinks con el UUID, sin accepted
bilinker check       → resuelve captures y compara contra accepted → cache/state
bilinker accept      → escribe accepted con el estado actual
bilinker apply       → repunta link a la ubicación nueva; deja RELOCATED
bilinker chain status <uuid> → inspecciona cadena completa
```

## `bilinker chain new`

### Crea una cadena: un UUID y un bilink en cada capa

```
bilinker chain new --tip <STRATUM_PATH[:LINE:COL[,LINE:COL]...]> \
                   [--mid <STRATUM_PATH>]... \
                   --tip <STRATUM_PATH[:LINE:COL[,LINE:COL]...]> \
                   [--kind <valor>] [--name.0 <etiqueta>] [--name.1 <etiqueta>] \
                   [--as.N <modo>] [--dry-run] [--yes]
```

| Argumento | Descripción |
|---|---|
| `--tip <ref>` | Extremo de la cadena: path Stratum al archivo con una o más posiciones, `abstract`, o `repo <alias>`. Exactamente dos veces. |
| `--mid <layer>` | Capa intermedia. Cero o más veces. |
| `--kind <valor>` | El [`kind`](bilink.md) del bilink. |
| `--name.N <etiqueta>` | El `name` del endpoint N. |
| `--as.N <modo>` | Qué parte del nodo señalado captura el tip N. Sin esto, el nodo entero. |
| `--as` | Sin valor: lista los modos disponibles y no hace nada más. |
| `--dry-run` | Muestra qué capturaría y no escribe nada. |
| `--yes` | No pregunta. Para scripts y para CI. |

Cada `--tip` captura el fragmento —sin posición, el archivo completo— y el endpoint queda apuntando a ese capture. Los mids llevan dos endpoints `path`. Ningún `accepted` se escribe: la cadena nace en `PENDING`.

Si un tip apunta a un fragmento ya capturado, el capture es literalmente el mismo archivo: el id sale de la ubicación.

```bash
bilinker chain new \
  --tip 'docs/specs/concepts/check.md:63:1' \
  --tip 'crates/bilinker/src/check.rs:405:1'
```

```
Created chain: 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a

  .bilink/7f3d8e9a-….yaml                    (tip)

Los dos endpoints quedan en PENDING. Revisar con `bilinker get` y aprobar con `bilinker accept`.
```

`--kind` existe para no depender de una edición a mano. `kind` y `name` son campos de declaración, y todo archivo de bilinker sale de un comando: sin el flag, la única forma de poblarlos sería abrir el YAML, que es justamente lo que el formato no pide de nadie.

### Un tip puede señalar varias partes

Las posiciones extra van separadas por coma, después de la primera:

```bash
bilinker chain new \
  --tip 'docs/specs/concepts/capture.md:66:1' \
  --tip 'crates/bilinker/src/query.rs:109:1,22:1'
```

Cada posición resuelve a su nodo igual que una sola —al ancla estable más cercana—, y de todas sale una query con un `@target` por nodo. El fragmento es su concatenación ([capture.md](capture.md), "El fragmento son los `@target`").

Las posiciones se descartan. Sirven para encontrar los nodos, y lo que se guarda es la query. El orden en que se pasan tampoco se guarda: el fragmento va en orden de archivo.

La query se ancla una sola vez. El nodo raíz del patrón es el ancla estable que contiene a todas las partes, así que las partes quedan ancladas entre sí: `@RequestMapping` de la clase que contiene al método, y no *"el primer `@RequestMapping` del archivo"*.

Si dos posiciones caen en el mismo nodo, es un nodo: no hay parte repetida.

### El path de un tip atraviesa directorios, no sólo capas

Un tip se escribe con tokens Stratum, y los tokens de navegación entre capas —`>name`, `<`— se mezclan con componentes de path corrientes:

```bash
bilinker chain new \
  --tip 'subsystems/bilinker/concepts/capture.md:29:1' \
  --tip 'subsystems/bilinker>impl/crates/bilinker/src/capture.rs:523:1'
```

`subsystems/bilinker>impl` es un directorio común y después una capa. Es lo mismo que el formato acepta en un endpoint `path` —`path subsystems/bilinker>impl`—, así que el comando no agrega una forma nueva sino que alcanza la que ya existe.

### Los dos tips de la frontera

Una cadena que cruza a otro proyecto se crea desde cada lado por separado, y no podría ser de otra manera: son dos repos y ninguno escribe en el otro.

El proveedor publica una punta abierta:

```bash
bilinker chain new \
  --tip 'src/main/java/…/UserPermissions.java:42:1' \
  --tip abstract
```

Un solo archivo, en su repo, con `link: abstract` del lado 1. Nadie más aparece: el proveedor no sabe quién va a consumirlo.

El consumidor referencia ese bilink por su UUID, que es el mismo:

```bash
bilinker chain new --from-repo hsi:8a3f0d21 \
  --tip 'src/permissions.ts:17:1'
```

`--from-repo <alias>:<uuid>` toma el UUID del bilink remoto en vez de generar uno nuevo, y arma el endpoint `repo` del otro lado. Es la única forma de `chain new` que no genera UUID: la convención de UUID compartido es lo que hace el rendezvous, y generar uno propio rompería el vínculo antes de crearlo.

El alias tiene que estar declarado —`.bilink/.hsi.toml`— y el clon tiene que estar. `chain new` sí puede clonar, a diferencia de `check`: es un acto explícito de una persona que está creando un vínculo ([frontier.md](frontier.md)).

## `--as`: quién genera la query

### El modo se pide; no se adivina

Sin `--as`, la query la genera el núcleo y captura los nodos señalados enteros. Con `--as <nombre>`, la genera un generador: `interface`, que es del núcleo, o un plugin como `spring-controller`.

Un generador sabe decir si tiene algo que decir sobre un nodo, y eso sirve para sugerir, nunca para elegir:

```
$ bilinker chain new --tip 'concepts/api.md:12:1' --tip '>impl/src/Ctl.java:16:5'
…
sugerencia: `--as.1 spring-controller` compone la ruta y deja el cuerpo afuera
```

Un generador que acierta cuando no querías ya te escribió otra cosa, y un capture es opaco después. Bilinker arregla solo lo que es suyo, y pide lo que es del repo de otro.

Va por tip, con la misma forma que `--name.N`, porque los dos extremos rara vez se capturan igual: del lado de la spec hay una sección de markdown y del lado del código un método, y un modo global obligaría a que el modo del código valiera también para la prosa.

Un generador toma una posición. Genera la query de eso que señalaste, y dos cosas señaladas son dos contratos, no uno con dos mitades. Sin `--as`, las posiciones siguen siendo las que quieras.

### Por qué es un nombre y no una flag booleana

Porque hay más de uno. `--as` toma un nombre, y eso hace que el atajo del núcleo y el plugin se pidan igual: `--as interface` y `--as spring-controller` son la misma forma. Un `--interface` booleano habría dejado a los plugins como ciudadanos de segunda.

`--as` sin valor lista los que hay.

### El capture no deja rastro, y el bilink sí

Un generador genera una query y desaparece. El capture que queda es una query normal: no dice quién lo generó, no depende de que el generador exista, y se podría haber escrito señalando las posiciones a mano.

Eso lo fuerza el formato, no la disciplina. El id de un capture es `sha256(file \0 query \0)`; agregarle *"generado por spring-controller"* le cambiaría el id sin cambiar la ubicación. No hay dónde dejar el rastro aunque uno quisiera.

El bilink es otro objeto, y ahí sí hay dónde. Su id es un UUID y ya lleva campos que no entran en ningún hash, así que el endpoint anota con qué se capturó en [`as`](bilink.md).

Y la mitad que importa se conserva entera: perder el plugin cuesta lo que el plugin sabía, nunca el vínculo. El capture sigue resolviendo, `check` sigue contestando, y un `as` que nombra un generador que no está instalado es un dato que no se pudo usar.

### Y pasa la misma verificación que una query escrita a mano

Que el capture resultante sea una query normal es lo que lo somete a la misma unicidad de la referencia que cualquier otra ([capture.md](capture.md), "Propiedades garantizadas de `capture`"). Un generador que produce una query que matchea más de un nodo no escribe, igual que `capture` sobre un ancla sin discriminante.

No es una regla nueva para generadores: es que no hay excepción. Un capture mal anclado reporta OK sobre una correspondencia que no existe, y eso no cambia porque lo haya escrito un plugin; cambia a peor, porque quien lo pidió no vio la query.

### `--as interface`: la firma sin el cuerpo

El atajo del caso común. Señalás el método y se captura su firma:

```bash
bilinker chain new \
  --tip 'concepts/api.md:12:1' \
  --as.1 interface --tip '>impl/src/Service.java:16:5'
```

Sin el atajo habría que señalar el tipo de retorno, el nombre y los parámetros por separado: tres posiciones para algo que el AST ya tiene agrupado.

### Lo que bilinker sabe, y es poco

Que en un nodo de función hay un campo que es el cuerpo, y que la firma es todo lo demás. En tree-sitter eso tiene nombre por gramática —`body` en Java, Rust y TypeScript—, y con eso alcanza: se capturan todos los hijos con nombre del nodo menos ése.

No es conocimiento de framework: es de la gramática, y la gramática ya es una dependencia. Es una tabla de la misma clase que las anclas estables, y existe por lo mismo, para que un lenguaje que no está falle en vez de adivinar:

```
$ bilinker chain new --as.1 interface --tip 'spec.md:1:1' --tip 'script.py:10:1'
Error: `--as interface` no sabe qué es el cuerpo en python.
       Señalar las partes a mano, o agregar python a la tabla.
```

Un nodo sin cuerpo se captura entero. Si la gramática no le da campo `body` —la firma de un método en una `interface` de TypeScript—, la firma es el nodo.

### El nombre se captura y además ancla

La firma incluye el nombre, y el nombre es además lo que la query usa para encontrar el nodo. Las dos cosas caen sobre el mismo nodo del AST y se escriben juntas:

```
(method_declaration
  name: (identifier) @n0 @target (#eq? @n0 "getPermissions")
  type: (generic_type) @target
  parameters: (formal_parameters) @target)
```

No es una redundancia que se pueda sacar. Sin el `@target`, renombrar el método no sería un cambio de contenido sino una relocalización, y el fragmento aceptado dejaría de mencionar cómo se llama lo que describe.

### `--as spring-controller`: el endpoint, no el método

Señalás sólo el método y el plugin va a buscar lo demás:

```bash
bilinker chain new \
  --tip 'concepts/api.md:12:1' \
  --as.1 spring-controller --tip '>impl/src/HSIPublicApiUsersRestImpl.java:16:5'
```

- sube a la clase y toma el `@RequestMapping`
- baja al método y toma su anotación de ruta —`@GetMapping`, `@PostMapping`, …
- toma el tipo de retorno y los parámetros

Cuatro fragmentos de una sola posición. Y con eso entra la ruta compuesta: sale de un `@RequestMapping` de clase más un `@GetMapping` de método, y el literal completo no aparece en ningún lado del archivo.

### El ancla es el nombre del método, y la ruta y el verbo son contenido

Una query generada tiene dos clases de cosas: lo que ancla —los predicados y la forma, que deciden si el fragmento se encuentra— y lo que se captura —los `@target`, cuyo texto entra en `hash`—. Un nodo en los dos roles hace que cambiarlo pierda el puntero en vez de mostrar el diff, y para un endpoint lo que tiene que poder cambiar y verse es la ruta, el verbo y la forma.

Así que el único predicado de nombre es el del método, y no lleva `@target`:

```
(class_declaration
  (modifiers
    (_
      name: (identifier) @n0 (#match? @n0 "^RequestMapping$")) @target)
  body: (class_body
    (method_declaration
      (modifiers
        (_
          name: (identifier) @n1 (#match? @n1 "^(GetMapping|PostMapping|PutMapping|DeleteMapping|PatchMapping|RequestMapping)$")) @target)
      name: (identifier) @n2 (#eq? @n2 "getPermissions")
      type: (_) @target
      parameters: (_) @target)))
```

Cambiar el literal de la ruta, el verbo o el prefijo de la clase deja el endpoint `ALTERED`, con su diff. Lo mismo sacarle el literal a la anotación o cambiar un `List<Dto>` por un `Dto`.

- **Las anotaciones se reconocen por clase, no por nombre.** `#match?` contra el conjunto de anotaciones de ruta dice *"la anotación de ruta del método"*, sea `@GetMapping` o `@PostMapping`; `#eq?` queda para el ancla. Por eso el último `#eq?` de la query es el nombre del método, que es lo que `check` muestra cuando un capture no resuelve y lo que `recapture` reescribe.
- **Los `@target` de contenido no fijan el kind.** `(_)` y no `(generic_type)`: el kind es parte de lo capturado, y un `annotation` que pasa a `marker_annotation` es la ruta que cambió, no un fragmento que desapareció.

Es el reparto inverso al de `--as interface`, que pone el nombre en los dos roles. Las dos reglas salen del mismo criterio —qué describe el fragmento—: una firma se describe por cómo se llama, y el contrato de un endpoint no incluye cómo se llama el método que lo sirve.

Lo que cuesta es que renombrar el método deja el capture sin ancla, y el endpoint `UNRESOLVED`. Cuando la similitud lo encuentra sin ambigüedad el capture es `REANCHORED`: `apply` lo repunta y el endpoint queda `RELOCATED` hasta que alguien acepte. Entre hermanos parecidos la similitud no alcanza, el capture es `UNANCHORED`, y la salida es `recapture`. No hay ancla más barata: en un endpoint cuya anotación no lleva literal no hay otra cosa que lo distinga de sus hermanos, y anclar por algo que el fragmento captura convierte el cambio que importa en un puntero perdido.

### El alias: el verbo y la ruta, compuestos del fragmento

Un bilink se identifica por su UUID, y para un endpoint hay un nombre que cualquiera reconoce. Está entero adentro de lo capturado, así que se compone y no se guarda:

```
GET /public-api/user/info/from-token
```

La ruta de clase y el literal del método son dos de los cuatro `@target`; el verbo sale del nombre de la anotación —`@GetMapping` → `GET`—. No hay que ir a buscar nada afuera del fragmento.

Y en un markerless hace falta el nombre del método. Sin literal propio, la ruta de clase y el verbo los comparten todos los hermanos, así que el alias sería ambiguo. Lo desempata el nombre, que sale de entre los dos últimos `@target`: el tipo de retorno termina, viene el nombre, arrancan los parámetros, y en el medio no hay nada más.

```
GET /public-api/appointment/booking/institution  ·  getBookingList
```

Donde falta el literal sobra el nombre, y viceversa.

Pero se lee del archivo y no de la query, aunque en la query esté. `name: (identifier)` aparece también en las anotaciones y en la clase, que van más arriba del árbol y por lo tanto antes en el patrón, así que ni el primero ni el último aciertan. Entre dos `@target` no hay ambigüedad posible: es la forma que el generador escribió, no una heurística sobre texto.

El alias es de cada generador y no del formato. Cada uno nombra en su vocabulario: acá es el verbo y la ruta porque eso es un endpoint; `--as interface` nombra por el método, porque eso es una firma. Un generador que no sepa nombrar no nombra, y el bilink se muestra por UUID.

Dónde vive el valor compuesto es de [la cache](cache.md), no de acá: es un derivado del capture, como `range`.

### Y bilinker no sabe de Spring

El plugin sí, y es todo lo que sabe: qué anotaciones marcan una ruta y dónde vive cada una. Está en un archivo, detrás del mismo trait que usa `interface`, y agregar otro framework es agregar otro archivo.

## La vista previa

### La salida deja ver qué se capturó, y qué no

Un capture es opaco después de escrito, así que una query mal generada se descubre tarde. Antes de escribir, `chain new` muestra cada tip que captura posiciones:

```
$ bilinker chain new --tip 'docs/spec.md:1:1' --tip 'src/Service.java:2:5,10:5'

. :: src/Service.java

     1   public class Service {
  ▸  2       public int uno(int a) {
  ▸  3           return a + 1;
  ▸  4       }
     5
     6       public int dos(int b) {
     ⋮
     8       }
     9
  ▸ 10       public int tres(int c) {
  ▸ 11           return c - 3;
  ▸ 12       }
    13   }

2 fragmentos · 2–4, 10–12
queda afuera: todo lo que no está marcado

¿escribir? [y/N/e(ditar)]
```

Cuatro cosas, y cada una atrapa un error distinto: el archivo como encabezado, una vez y no repetido por parte; contexto alrededor, con `⋮` donde se saltan líneas; `▸` sobre lo capturado, así lo que no entra se ve sin marcar; y una línea que dice qué quedó afuera, porque es lo que más se malinterpreta.

El error que esto atrapa es señalar el nodo equivocado en un archivo con veinte parecidos: se ve porque la línea marcada queda lejos de donde tenía que estar.

Con `--dry-run` se muestra lo mismo y no se escribe nada, ni el capture ni el bilink. Con `--yes` no se pregunta.

Sin terminal tampoco se pregunta. Un `chain new` adentro de un script no puede quedarse esperando una tecla que nadie va a apretar; la vista se imprime igual, por stderr, y se escribe. La confirmación existe para la persona que está mirando, y `--yes` es cómo se dice eso explícitamente.

### Y las marcas se editan

Confirmar con `y/N` obliga a volver a empezar cuando la resolución agarró mal. `e` abre la misma vista en el editor, y ahí se corrige: se saca un `▸`, se pone otro, se guarda.

Las marcas son señales, no rangos. Cada línea marcada resuelve a su nodo, igual que una posición de la línea de comandos, así que editar el buffer es otra forma de señalar y lo que se guarda sigue siendo la query. Marcar tres líneas de una función marca la función una vez.

Los dos tips van en un solo buffer. Abrir un editor por tip haría corregir a ciegas el segundo, y lo que se está revisando es el vínculo, no cada punta por su cuenta. Al guardar, la vista corregida se vuelve a mostrar: la corrección también se revisa.

Un buffer que vuelve sin ninguna marca no escribe nada: es la forma de abortar, la misma que `git commit` con un mensaje vacío.

El editor es el de git, resuelto con `git var GIT_EDITOR`: contesta lo que git realmente usaría, respetando `$GIT_EDITOR` → `core.editor` → `$VISUAL` → `$EDITOR` → el fallback del sistema. Es el mismo criterio por el que quién acepta sale de `git var GIT_AUTHOR_IDENT` y no de `user.name`. Si git no contesta, queda el `y/N`.

## `bilinker chain status` y `bilinker chain list`

### `bilinker chain status <uuid>` recorre todos los nodos

```
$ bilinker chain status 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a

Chain: 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a  [DIRTY]

  .bilink/                         (tip)   (OK, CHAIN_DIRTY)
    link.0  specs :: voting.yaml#impl       OK
    link.1  → .stratum/impl                CHAIN_DIRTY

  .stratum/impl/                   (tip)   (CHAIN_DIRTY, ALTERED)
    link.0  → spec layer                   CHAIN_DIRTY
    link.1  java-demo :: Persona#vote      ALTERED
              source: commit c7d3e9f "Inline comparator"
```

El estado global de la cadena es el de "El estado de la cadena es el peor de sus nodos".

### `bilinker chain list` lista las cadenas a partir del directorio actual

```
bilinker chain list [<texto>] [--as <modo>] [--link <tipo>] [--state <estado>] [--under <path>]
```

| Argumento | Filtra por |
|---|---|
| `<texto>` | que el alias lo contenga, sin distinguir mayúsculas |
| `--as <modo>` | con qué generador se capturó algún extremo: `spring-controller`, `interface` |
| `--link <tipo>` | qué clase de extremo tiene: `capture`, `path`, `issue`, `abstract`, `repo` |
| `--state <estado>` | el estado de la cadena: `ok`, `pendiente`, `dirty`, … |
| `--under <path>` | que algún extremo referencie un archivo bajo ese path |

```
$ bilinker chain list

7f3d8e9a  [DIRTY]   GET /public-api/user/info/from-token
3a4b5c6d  [OK]      GET /public-api/appointment/booking/institution  ·  getBookingList
f1e2d3c4  [BROKEN]  spec → impl
```

Cada cadena se nombra por su alias si alguno de sus extremos sabe nombrarse. Con 98 endpoints un listado de hexadecimales no distingue nada de nada, y encontrar uno obliga a sacar cada UUID y correrle `get`.

Y el que no tiene alias se muestra por UUID. Un extremo sin `as` no tiene generador que lo nombre. No es un error: es el estado de casi todo, y el listado tiene que seguir sirviendo ahí.

### Los filtros se combinan con Y

Un `chain list booking --state pendiente` es *las que se llaman así y están sin aceptar*; que se acumulen es lo que permite bajar de 98 a una sin salir del comando.

Una línea por cadena alcanza mientras haya diez. Con 98 —la superficie pública de un proveedor real— encontrar una obligaba a correr `get` uno por uno. En un repo propio el problema queda escondido, porque uno encuentra un bilink por el archivo que está editando, con `get <file>`. Del otro lado de la frontera nadie sabe qué archivo mirar.

### Los dos ejes de tipo no son el mismo, y por eso son dos flags

| eje | flag | valores | contesta |
|---|---|---|---|
| tipo del `link` | `--link` | `capture`, `path`, `issue`, `abstract`, `repo` | qué clase de extremo es |
| generador | `--as` | `spring-controller`, `interface`, ausente | con qué receta se capturó |

Son independientes: un `capture` puede tener cualquier `as` o ninguno, y un `abstract` no tiene ninguno porque no captura nada.

`--as` es el que hace útil un listado en un repo mezclado, donde conviven secciones de markdown, funciones de Rust y firmas capturadas con `interface`: pedir *"los `spring-controller`"* es pedir una clase de cosa, no un texto que aparezca en un nombre.

### Y `abstracts` sin alias es esto con `--link abstract`

| | qué lista | de dónde lo lee |
|---|---|---|
| `chain list --link abstract` | lo que esta capa publica | sus propios `.bilink/` |
| `abstracts` sin alias | lo mismo | ídem: es este comando con el filtro puesto |
| `abstracts <alias>` | lo que publica otro repo | el clon del proveedor, con `git show` y sin tocar el sparse |

La tercera fila contesta una pregunta que `chain list` no puede: mira el repo de otro ([frontier.md](frontier.md)).

El formato de cada uno sigue distinto porque las preguntas son distintas. `abstracts` trae el fragmento porque elegir de qué colgarse se decide leyendo el código; `chain list` trae el alias porque encontrar una entre 98 se hace por nombre.

### Código de salida de `chain`

| Código | Condición |
|---|---|
| 0 | Operación exitosa. |
| 1 | Error: UUID no encontrado, capa inválida, UUID duplicado en una capa. |

## `bilinker remove`

### Elimina el bilink de la capa actual

```
bilinker remove <uuid>
```

1. Resuelve `.bilink/<uuid>.yaml` en la capa actual, en el árbol o, si ya no está ahí, en la ref.
2. Elimina el archivo.
3. Commitea el borrado en `refs/bilink/<branch>`.
4. No elimina los captures que referenciaba: pueden estar en uso por otros bilinks. Un capture que queda sin referentes se limpia con `bilinker capture prune`.
5. Los nodos adyacentes de la cadena detectarán `BROKEN` en el próximo `check` y deberán decidir: reparar o también remover. La remoción se propaga hop a hop, no es automática.

```
removed: .bilink/7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.yaml
commit:  refs/bilink/… @ 4c1d9e0

note: nodos adyacentes detectarán BROKEN en el próximo check
note: 1 capture quedó sin referentes — `bilinker capture prune` para limpiarlo
```

### El borrado es un commit propio en la ref

Un commit de tipo decisión, de un padre, cuya primera línea es `remove <uuid>` ([ref.md](ref.md)). Su árbol es el del commit anterior de la ref menos ese bilink, y no el `.bilink/` del árbol de trabajo: otro cambio sin commitear en `.bilink/` no entra en el commit, y sigue en `bilinker diff`.

`bilinker push` lo publica como cualquier otra decisión. En una capa que todavía no cortó a la ref, `remove` sólo borra el archivo, y commitearlo es de quien trabaja.

### Un borrado que sólo está en el árbol se publica con el mismo `remove`

Si el bilink ya no está en el árbol y sigue en la ref —un borrado hecho con un binario anterior—, `remove <uuid>` commitea el borrado igual. Si no está en ninguno de los dos, es un error.

```
$ bilinker remove 35876ceb
removed: .bilink/35876ceb-7df1-4406-bb59-f61926c0267a.yaml  (ya no estaba en el árbol)
commit:  refs/bilink/… @ 7e21a3f
```

### `remove` es para lo que ya no tiene sentido

Para los estados `DELETED` y `BROKEN` donde el bilink ya no tiene sentido: el fragmento fue eliminado definitivamente, el repo fue removido, o el bilink fue creado por error. No es un sustituto de `bilinker accept`: si el fragmento cambió pero sigue siendo válido, corresponde `accept`.

| Código | Condición |
|---|---|
| 0 | Archivo eliminado. |
| 1 | UUID no encontrado. |
