# La navegación

`get` permite navegar desde una posición en un archivo hacia los fragmentos relacionados a través de bilinks, y recuperar su contenido. Opera en tres formas.

## Forma 1: posición → endpoints que la cubren

### `get <file>:<line>:<col>` lista los endpoints cuyo capture cubre la posición

```
bilinker get <file>:<line>:<col>
```

| Argumento | Tipo | Descripción |
|---|---|---|
| `file` | path | Path al archivo (absoluto o relativo al directorio de trabajo). |
| `line` | int | Línea (1-based). |
| `col` | int | Columna (1-based). |

Retorna la lista de endpoints de bilinks cuyo capture tiene un `range` que cubre la posición dada. Cada resultado se identifica como `<UUID>.<N>` y muestra una descripción del fragmento referenciado en el extremo opuesto.

Usa `.bilink/index/index` si está disponible y actualizado (O(1)); si no, escanea los bilinks de la capa actual (O(N)).

```
$ bilinker get src/main/java/ar/example/demo/persona/Persona.java:11:5

7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1   specs :: voting.yaml#impl
3a4b5c6d-2e3f-4a5b-9c6d-7e8f9a0b1c2d.1   specs :: persona/voting.yaml#impl
```

Si no hay bilinks que cubran esa posición, retorna lista vacía (código 0).

### Un endpoint sin rango en la cache no se descarta en silencio

Sin rango no se puede decir si cubre la posición. `get` no lo lista, y dice por stderr cuántos endpoints del archivo no tienen rango y que hay que correr `bilinker check .`: una lista vacía con ese aviso no afirma que nada cubra la posición.

## Forma 2: endpoint → contenido del fragmento referenciado

### `get <UUID>.<N>` devuelve el texto del fragmento que el endpoint referencia

```
bilinker get <UUID>.<N> [-B <rows>] [-A <rows>] [--diff] [--raw]
```

| Argumento | Tipo | Descripción |
|---|---|---|
| `<UUID>.<N>` | string | Identificador del endpoint: UUID de la cadena + índice (0 o 1). |
| `-B rows` | int | Líneas de contexto antes del fragmento. |
| `-A rows` | int | Líneas de contexto después del fragmento. |
| `--diff` | flag | Muestra el diff entre el fragmento aceptado y el fragmento actual. |
| `--raw` | flag | El texto del fragmento y nada más: sin números de línea y sin huecos. |

Resuelve el endpoint N del bilink `<uuid>.yaml` de la capa actual y retorna el texto del fragmento que referencia.

Si el endpoint es de tipo `path`, resuelve el path Stratum hacia la capa adyacente, localiza el mismo UUID en su `.bilink/`, y retorna el fragmento del endpoint estructural que contiene. Requiere que los archivos de la capa adyacente estén accesibles localmente.

stdout lleva el fragmento, una línea por línea del archivo, con su número. stderr lleva la metadata:

```
# specs :: persona/voting.yaml  lines 14–16
```

Con [alias](chain.md), el encabezado lo lleva: es lo que contesta qué se está mirando, y el archivo y las líneas contestan dónde.

```
# GET /public-api/user/info/from-token
# hsi :: …/UserPublicController.java  lines 22–22, 36–37
```

Un fragmento de varios `@target` se muestra por partes, y la metadata lleva un tramo por parte.

```
$ bilinker get 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1 -B 2 -A 2

12:     id: persona-voting
13:     name: Voting
14:   impl: Persona#vote
15:   description: El método vote registra el voto del ciudadano.
16:
17:   tests:
```

### Un hueco es un hueco, y se marca en las dos escalas

`⋮` para las líneas que no entran. `...` para lo que no entra adentro de una línea.

```
$ bilinker get 67ba7217.0

22:   @RequestMapping("/public-api/user")
 ⋮
36:   	@GetMapping(value = "/info/from-token")
37:   	... PublicUserInfoDto ... (@RequestHeader("user-token") String userToken) ...
```

Los `...` son el límite entre partes, y por eso no hace falta marcarlo aparte. En la línea 37 dicen tres cosas de una: que `public` no entra, que el nombre del método no entra, y que el ` {` tampoco. Dónde termina una parte y arranca la otra se ve porque lo que hay en el medio no está.

Y hace falta porque el texto pelado miente. Todo capture de `spring-controller` tiene cuatro `@target`, y el tipo de retorno y los parámetros comparten línea siempre que la firma quepa en una. Concatenados, esa línea sale dos veces y se lee como una duplicación que no existe. Con la vista, una línea del archivo sale una vez, aunque la toquen varias partes.

La sangría se conserva tal cual: es espacio en blanco, no aporta contenido, y sin ella el código no se lee.

### `--raw` es el texto, y no es el default

Con `--raw` el stdout es el fragmento exacto, unido por el mismo separador que lo une en el `hash`: lo que se imprime es el fragmento, el mismo que `check` compara.

El argumento no es de gustos. Si alguien quiere el texto exacto es para compararlo, y comparar lo hacen `check` y `--diff`, no una persona en una terminal. Un default que sirve para pipear y no para leer optimiza el uso raro.

Y `get` es el comando de después. La [vista previa](chain.md) existe porque un capture es opaco después de escrito; `get` se usa justo cuando esa opacidad ya se cobró, y no puede mostrar menos que la vista de antes de escribir.

Y el vecindario tampoco entra en `--raw`, por lo mismo: lo que `check` hashea como fragmento es el fragmento, y un vecino adentro rompe la única salida de la que se puede afirmar que es byte por byte.

### El vecindario viene con el fragmento

Un endpoint con [vecindario de nivel 1](accept.md) no está aprobado por su fragmento solo: `accept` escribe `hash` y `n` en la misma entrada, y no existe el camino *"apruebo la firma y el vecindario no lo miré"*. `get` es el comando con el que se mira antes de aprobar, así que trae las dos partes.

Sin eso el comando muestra la firma y calla los tipos que la firma menciona, que es exactamente la mitad que el nivel 1 existe para cubrir. Y en `CONTRACT_ALTERED`, que es el caso donde el fragmento no cambió y aun así el contrato se movió, lo único que `get` imprimiría es lo que sigue igual.

```
$ bilinker get 67ba7217.0

# GET /public-api/user/info/from-token
# hsi :: …/UserPublicController.java  lines 22–22, 36–37
22:   @RequestMapping("/public-api/user")
 ⋮
36:   	@GetMapping(value = "/info/from-token")
37:   	... PublicUserInfoDto ... (@RequestHeader("user-token") String userToken) ...

# n1 · hsi :: …/dto/PublicUserInfoDto.java  lines 8–13
 8:   public class PublicUserInfoDto {
 9:       private String username;
10:       private Long authorityId;
11:       private AuthorityKind authorityKind;
12:       private Instant expiresAt;
13:   }
```

Cada vecino es un capture, así que se muestra como cualquier otro fragmento. Los números de línea, el `⋮`, los `...` y la regla de que una línea del archivo sale una sola vez son de la vista y no del endpoint: lo único que cambia es qué capture se resuelve.

De ahí que cuánto se trae de un vecino no sea una pregunta de este comando. Un capture nombra un nodo entero y no hay sub-rango, así que un recorte de vecino le inventaría al formato una operación que no tiene en ninguna parte. Lo mismo con el orden: los ids de `n.1.link` están ordenados por identidad, que es el orden del fold, y `get` no los reordena.

Cada vecino lleva su propio encabezado, en el mismo lugar donde va el del fragmento —stderr— y con el mismo formato más el prefijo del nivel. Es lo que impide que el fragmento y sus vecinos se lean como uno solo.

### Se traen los declarados, no los aceptados

`n.1.link` del endpoint es la declaración —los vecinos de hoy, que mantiene `apply`— y `accepted[].n.1.link` es la decisión. `get` lee la declaración, por la misma razón por la que el fragmento que imprime es el de hoy y no el aceptado: contrastar contra lo aceptado es `--diff`.

Cuando los dos conjuntos difieren el estado es `CONTRACT_RELOCATED`, y lo que hay que mirar para decidir si se acepta es justamente el de hoy.

### No pide proveedor, y por eso puede ser el default

Los vecinos ya son captures escritos en el bilink. Traerlos es resolverlos con tree-sitter sobre archivos de la misma capa: la misma operación que `get` ya hace con el capture del endpoint.

Así que la regla de que todo comando que toque el eje del vecindario recibe el puerto y degrada sin él no le aplica: `get` no recalcula el vecindario, lee el que está escrito. Mostrarlo cuesta abrir N archivos más, donde N son los tipos que la firma menciona, y no cuesta una sola pregunta a un language server.

### Cuando no hay vecinos que traer, no es un solo caso

Cuatro formas de no tener nada que imprimir, y una sola es un silencio legítimo:

| El archivo dice | Qué pasó | Qué imprime |
|---|---|---|
| ni declaración ni contrato aceptado | el fragmento no tiene firma resoluble: prosa, un DTO, un `enum` | nada |
| `n.1` sin [`link`](bilink.md) | se preguntó y no hay vecinos de esta capa | `# n1 · sin vecinos` |
| hay contrato aceptado y no hay declaración que lo ubique | el contrato está y de qué vecinos salió no se sabe | `# n1 · ubicación desconocida — acuñar con bilinker apply` |
| `n: declined` | alguien renunció al vecindario | `# n1 · renunciado` |

La primera es la única donde no imprimir no afirma nada: no hay vecindario porque no hay firma, y decirlo debajo de cada fragmento de prosa sería ruido en la salida más común del comando.

La tercera no se lee sólo en `n.1.link: unknown`: un `accepted` con contrato cuya declaración no está es el mismo caso —`CONTRACT_UNLOCATED`— y llega sin ningún `unknown` escrito, porque el `unknown` vive del lado de la decisión. Mirar sólo la declaración hace que un fragmento con firma y contrato salga como uno de prosa. Que el nivel aceptado tenga ids o diga `unknown` no cambia lo que hay que hacer —la declaración es la que falta, y la escribe `apply`—, así que no cambia lo que se dice.

Las otras tres tienen contrato o lo tuvieron, y un silencio las volvería indistinguibles de la primera: *"no había nada"*, *"no se pudo"* y *"se renunció"* no se dicen igual.

### Un vecino que no resuelve se imprime igual, y no hace fallar al comando

Vale la regla de "Cuando el capture no resuelve, igual dice a dónde apuntaba" —archivo, id del capture, estado y query— porque el problema es el mismo y el dato que hace falta para arreglarlo también.

Lo que cambia es el código de salida: sigue siendo 0, porque el fragmento que se pidió salió. El estado de un capture es de `check`, y hacer fallar a `get` por un vecino roto le negaría al que mira el fragmento que sí resolvió.

### No hay flag para apagar el vecindario

El fragmento solo ya se pide, y es `--raw`. Un segundo flag —el fragmento numerado y sin vecinos— existiría para un uso que no aparece: el que corre `get` está por aceptar, y lo que acepta es el contrato entero.

### `--diff` compara el fragmento aceptado con el actual

Requiere el `commit` del endpoint —el commit en que el contenido aceptado quedó establecido— y el capture resuelto. Vive en [la cache](cache.md); con cache fría se re-deriva.

- "antes": el fragmento aceptado, recuperado resolviendo la query contra el contenido del `commit` del endpoint y verificado contra `accepted.hash` ([check.md](check.md), "Recuperar el texto aceptado").
- "después": resuelve el fragmento actual con la misma query AST que usa `get` normalmente.
- Muestra un unified diff del fragmento, sin contexto extra de archivo.

No se recorta el contenido viejo por el `range` del capture: ese range es la posición actual, que `check` reescribe en cada corrida, así que aplicarlo a contenido de otro commit da bytes arbitrarios.

Si la verificación por hash falla —el archivo no existía en ese commit, la query no resuelve ahí— `--diff` cae al recorte por range. Para un diff informativo mostrar algo aproximado es mejor que no mostrar nada; el contraste es con `check`, que ante la misma duda prefiere no detectar, porque de ahí salen decisiones.

```
$ bilinker get 7f3d8e9a.1 --diff

# java-demo :: src/main/java/ar/example/demo/persona/Persona.java  lines 10–12
--- aceptado (commit a3f2b1c)
+++ actual
@@ -1,3 +1,4 @@
 public void vote(String candidato) {
-    repo.save(new Vote(candidato));
+    repo.save(new Vote(candidato, true));
+    audit.log(candidato);
 }
```

Si el fragmento no cambió (estado OK), muestra el contenido sin diff.

### Cruzando la frontera, el clon se profundiza a pedido

Un endpoint [repo](frontier.md) apunta a un fragmento de otro proyecto, y su clon arranca superficial: el árbol actual de la rama declarada, sin historia. Alcanza para `check`, que no puede andar profundizando clones como efecto colateral.

`get --diff` es el que sí necesita historia, y la trae sólo para ese bilink:

```
1. Recorrer el .bilink remoto hacia atrás por la ref del proveedor,
   hasta la versión cuyo `accepted` coincide con lo que este repo aceptó.
2. Leer el capture de entonces y el archivo de entonces.
3. Comparar contra lo que el proveedor publica ahora.
```

El paso 1 es lo que hace que ningún commit del proveedor se copie: se descubre, no se guarda. Y el fetch es incremental —`--deepen`, o los blobs de un clon parcial— así que el costo se paga únicamente donde hay un humano mirando.

Ése es el reparto: `check` es masivo y barato; ver el diff es puntual y caro. El conocimiento mínimo queda como default, no como límite.

Si el clon no está, `--diff` no lo crea: dice que falta y sale, igual que `check` reporta `REMOTE_UNREACHABLE`. Traer un repo ajeno por primera vez es un acto explícito.

`--diff` opera solo sobre el endpoint pedido. El traversal del call graph vive en lattice: bilinker no consulta language servers para eso.

## Forma 3: archivo → todos los endpoints que lo referencian

### `get <file>` lista todo endpoint que referencia el archivo

```
bilinker get <file>
```

| Argumento | Tipo | Descripción |
|---|---|---|
| `file` | path | Path al archivo (absoluto o relativo al directorio de trabajo). |

Retorna todos los endpoints de bilinks que referencian ese archivo, ya sea mediante un capture con query AST, un capture de archivo completo, o cualquier posición dentro del archivo.

Usa `.bilink/index/index` si está disponible y actualizado (O(1)); si no, escanea los bilinks de la capa actual (O(N)). En ambos casos, no re-ejecuta queries tree-sitter: el `range` cacheado de cada capture es suficiente.

```
$ bilinker get src/main/java/ar/example/demo/persona/Persona.java

7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a.1   specs :: voting.yaml#impl          lines 11–18
3a4b5c6d-2e3f-4a5b-9c6d-7e8f9a0b1c2d.1   specs :: persona/voting.yaml#impl  lines 11–18
```

Si no hay bilinks que referencien ese archivo, retorna lista vacía (código 0).

### Con la cache fría, lista igual los endpoints del archivo

Qué endpoints referencian un archivo lo dicen los bilinks y sus captures, no la cache: la cache sólo aporta el rango. Un endpoint sin rango en la cache, como uno aceptado después del último `check`, sale en la lista con `sin rango` en lugar de los bytes, y stderr dice que hay que correr `bilinker check .`.

```
$ bilinker get src/lib.rs

4d6f25b7-118b-41a5-9e9b-fb54ce894cff.1  capture db95cb9cf71e45dec3b8a3159ca7424c  sin rango
790d32a2-7815-496e-ba60-3a932cf60c1b.1  capture 277c72f4cd0f27e63e22f93dce9c0ab5  bytes 0~11
hay endpoints sin rango: la cache no los tiene.
  Correr `bilinker check .` para calcularlos.
```

## Cuando el capture no resuelve

### Cuando el capture no resuelve, igual dice a dónde apuntaba

Un endpoint que no resuelve no tiene contenido que mostrar. El estado ya dijo que el fragmento no está; lo que hace falta para arreglarlo es cuál era, para decidir a dónde repuntarlo. Y `UNRESOLVED` es el estado que obliga a intervenir a mano: no tiene fix automático y no se resuelve aceptando.

Así que la referencia se imprime igual, y el error va después:

```
$ bilinker get be2e3fd6.1
# crates/bilinker/src/apply.rs
# capture 1a2b3c4d…  (UNANCHORED)
query: (function_item name: (identifier) @n0 (#eq? @n0 "compute_fix")) @target

Error: el anchor `compute_fix` no está en el archivo.
  Repuntar con `bilinker recapture be2e3fd6.1 <archivo> <línea>:<col>`.
```

Sigue fallando —el código de salida es 1— porque no hay fragmento que devolver. Lo que cambia es que la salida no obliga a abrir el `.yaml` del capture para leer la query, que es exactamente lo que el formato evita pedirle a nadie.

Es la misma regla que gobierna [`apply`](apply.md) al no poder calcular un fix: la salida es fiel a lo que la herramienta sabe. El capture está ahí, se leyó para intentar resolverlo, y lo que se imprime lo incluye.

### La causa se re-deriva, no se supone

*"La query no matcheó"* es cierto y no sirve: es la observación, no la causa. Así que el estado del capture se vuelve a resolver, y de ahí sale qué comando corresponde:

| Estado | Qué pasó | Qué hay que hacer |
|---|---|---|
| `MOVED` | el archivo se movió | `bilinker apply` |
| `REANCHORED` | el anchor se renombró y se localizó por similitud | `bilinker apply` |
| sin fix, y el anchor aparece en un archivo sin trackear | git no puede ver el rename | `git add` ese archivo |
| sin fix, y el anchor no aparece en ninguna parte | el fragmento ya no existe | `recapture` o `remove` |

La tercera fila es la que `apply` no puede explicar: sin rename detectado el capture queda en un estado sin fix, y `apply` no lo mira. Se decide buscando el anchor entre los archivos sin trackear: un hecho, no una sugerencia genérica.

## Código de salida y propiedades

### Código de salida de `get`

| Código | Condición |
|---|---|
| 0 | Operación exitosa (puede haber 0 resultados en forma 1 y 3). |
| 1 | Error: archivo no encontrado, UUID inválido, endpoint sin capture, el capture del fragmento sin resolver, capa adyacente no accesible. |

Un vecino que no resuelve no entra en esa lista. El fragmento pedido salió, y lo que no resolvió se imprimió con su referencia.

### Propiedades garantizadas de `get`

- Independencia de git: `get` sin `--diff` no requiere control de versiones.
- Sin efectos secundarios: `get` no escribe ningún archivo.
- Un endpoint que no resuelve imprime su referencia igual: archivo, id del capture, y query. Falla después.
- `--raw` imprime el fragmento y nada más: el mismo texto que `check` hashea, byte por byte. Es la única salida de la que eso se puede afirmar, y por eso el flag existe.
- Una línea del archivo sale una sola vez, aunque la toquen varias partes.
- El vecindario declarado sale con el fragmento, en el orden en que está escrito en `n.1.link` y con la misma vista, y cada vecino con su encabezado.
- `get` no le pregunta al proveedor de vecindario: los vecinos son captures del bilink, y traerlos es resolverlos.
- Un vecindario que no se puede traer se nombra, y sólo la ausencia de firma resoluble es silencio.
