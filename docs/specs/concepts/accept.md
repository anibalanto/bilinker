# La aceptación

Aceptar es decir *"revisé esto y lo apruebo"*. Es el único acto del sistema que no se puede derivar de nada, y por eso es lo único —junto con la declaración— que queda versionado.

## Las dos dimensiones

### Ubicación y contenido se aprueban por separado

Un endpoint puede desalinearse de dos formas distintas, y hay que poder aprobarlas por separado:

| Dimensión | Cambió | Se aprueba |
|---|---|---|
| Ubicación | el fragmento se movió: otro archivo, otro nodo | `accepted.link` |
| Contenido | el fragmento sigue donde estaba y dice otra cosa | `accepted.hash` |

`apply` propone la ubicación nueva pero no la bendice. El endpoint queda en `RELOCATED` hasta que alguien acepte. `apply` propone, `accept` dispone: es la misma división que hay entre corregir y aprobar, extendida a la dimensión de la ubicación.

### `--place` y `--content`

Por defecto `accept` aprueba las dos dimensiones. Los dos flags permiten aprobar una sola:

```sh
bilinker accept <uuid>.<N>              # ubicación y contenido
bilinker accept <uuid>.<N> --place      # sólo la ubicación
bilinker accept <uuid>.<N> --content    # sólo el contenido
```

Aceptar la ubicación de un fragmento cuyo contenido no se revisó es una situación real —el archivo se renombró y el código no cambió, o cambió y hay que mirarlo aparte— y sin los flags habría que aprobar de más para poder avanzar.

Vale también para `agree`: lo que se compara es el par, no cada dimensión por su lado. Aprobar una ubicación nueva sobre un contenido que nadie volvió a mirar produce un par que nadie más aprobó.

## El bloque

### Los campos de una entrada

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
          hash: 96c765b9a3f1e4d7c2b8a5f0e3d6c9b2a7f4e1d8c5b0a3f6e9d2c7b4a1f8e5d0
          hash_ast: 88e834c4b1a7f2e5d0c3b6a9f4e7d2c5b8a1f6e3d0c7b4a9f2e5d8c1b6a3f0e7
  1:
    link: path >impl
```

| Campo | |
|---|---|
| `agree` | Quiénes aprobaron estos valores. Ver "Quiénes aprobaron". |
| `link` | La ubicación aprobada. Ausente en un endpoint `issue` o `abstract`, que no tienen capture. |
| `hash` | SHA-256 del fragmento aprobado: la concatenación de los `@target` ([capture.md](capture.md)). |
| `hash_ast` | SHA-256 de su s-expression, y de las de todos sus nodos unidas por `\n` cuando hay más de uno. Opcional: ausente donde no hay gramática. |
| `n` | El vecindario, por nivel: un capture por vecino y sus dos folds. Tres estados, ver abajo. |

`accepted` es una lista, y el eje del vecindario tiene la misma forma que el del fragmento: declaración afuera, decisión adentro. Por qué más de una entrada es un estado y cómo colapsa está en [bilink.md](bilink.md).

`hash` y `hash_ast` van separados y no hasheados juntos porque `RESTYLED` necesita compararlos por separado. Donde `hash_ast` no está, `RESTYLED` no existe y todo cambio de texto es `ALTERED`, que en prosa es lo correcto.

### `accepted` está o no está

Su ausencia es `PENDING`, literalmente: no hay que enunciar que los campos de aceptación están presentes juntos o ausentes juntos, porque el bloque no se puede escribir a medias. Lo verifica el tipo: `accepted` sin `hash` es rechazado, y un `hash` suelto afuera del bloque también.

### `n` es un campo con tres estados

El vecindario tiene una respuesta, y por eso es un campo, con los niveles adentro:

```yaml
n:                       # se adquirió
  1:
    link: capture fe74f8b4… 1b06e7c6…   # un capture por vecino
    hash: 96c765b9…
    hash_ast: 88e834c4…  # opcional adentro del nivel, y todo-o-nada

n: declined              # alguien renunció, a propósito

                         # ausente: el fragmento no tiene firma resoluble
```

Escrito como campos sueltos —`hash_n1`, `hash_ast_n1` y una marca aparte— quedaban representables combinaciones que no significan nada: un fold de ASTs sin el fold de textos que lo acompaña, y una renuncia conviviendo con el valor al que se renunció. Que ningún código las produzca no es lo mismo que que no se puedan escribir, y el YAML lo escribe cualquiera a mano.

Plegado, `hash_ast` no puede estar sin su `hash` porque vive adentro del mismo fold, y `declined` no puede convivir con un nivel porque son variantes del mismo campo.

### La renuncia va en el contenedor, no adentro de un nivel

Es una sola decisión, y es de 1 para arriba. El nivel 2 son los campos de los tipos que el 1 resuelve, así que está definido a través del 1: renunciar al 1 deja al 2 sin base sobre la cual existir. Y por eso no hay un `--no-n2` que tenga sentido solo: el corte que la renuncia nombra no es un número de nivel, es lo que bilinker calcula por su cuenta contra lo que pide un language server, y el 1 es el primero del segundo lado.

Escribirla adentro del nivel 1 —`n1: declined`— diría *"el nivel 1 fue renunciado"* cuando quiere decir *"el vecindario fue renunciado"*. En el contenedor esa pregunta no llega a existir, porque no hay dónde escribir la respuesta equivocada.

### El nivel 0 no entra

`hash` y `hash_ast` —el fragmento— se quedan afuera del mapa, aunque se hasheen igual que un vecino:

| | nivel 0 | niveles ≥1 |
|---|---|---|
| es obligatorio | sí: `accepted` sin `hash` se rechaza | no |
| se puede renunciar | no: aceptar es hashearlo | sí |
| de dónde sale | tree-sitter y git, siempre | un language server, que puede no estar |

Adentro del mapa, `n: {}` sería una aceptación sin contenido aprobado y `n: {0: declined}` sería escribible sin querer decir nada. La regularidad se paga con dos estados inválidos, y no vale.

## El cierre de firma

Un fragmento no es sólo un árbol de sintaxis: devuelve un tipo. Con el capture de contrato de [capture.md](capture.md), la firma entra en `hash` y un cambio de tipo de retorno se detecta. Lo que no se ve es que el tipo siga llamándose igual y tenga otros campos, que es exactamente lo que rompe a un consumidor. `n` cubre esa fila, y una sola: el vecindario de nivel 1.

### No recursa, y está en el nombre

Los vecinos son los tipos que la firma menciona, un salto. Los campos de esos tipos son nivel 2 y no entran.

| Cambio en el proveedor | ¿Lo ve? |
|---|---|
| al DTO le agregan un campo | sí: cambió su texto |
| `String name` → `AuthorityKind name` en el DTO | sí |
| el DTO se muda de archivo | sí: cambió el conjunto |
| el método devuelve otro tipo | sí: cambió el conjunto |
| a `AuthorityKind` le cambian los valores | no: es nivel 2 |

Clavar la profundidad en 1 retira tres preguntas de una vez: dónde para el cierre, cómo termina con tipos recursivos, y cómo se recorre. No hay recorrido: se resuelven los tipos de la firma y listo. Si alguna vez hace falta más, un nivel 2 es aditivo y no invalida nada.

### Dónde se pregunta: los identificadores de tipo, no el primer byte de cada campo

Los campos de la firma que llevan tipos —`type` y `parameters` en java, `return_type` y `parameters` en rust y typescript— dicen dónde buscar, y no son ellos las posiciones. Lo que recibe el puerto son los identificadores de tipo que esos campos contienen, cada uno en su byte inicial.

Tomar el primer byte de cada campo parece equivalente, y falla porque un campo no empieza en un tipo:

| El campo | Su primer byte | Qué declara |
|---|---|---|
| `parameters` | `(` | nada, y go-to-definition sobre un paréntesis contesta la función que lo contiene |
| `return_type` de `Result<Checked>` | `Result` | el tipo de más afuera, casi siempre de otra capa: se descarta, y `Checked` no se pregunta nunca |
| `type` de `ResponseEntity<List<Dto>>` | `ResponseEntity` | lo mismo, con el `Dto` dos niveles adentro |

El de `parameters` es el más grave de los tres: los otros dos dejan un vecino afuera; éste mete uno falso, el fragmento se declara vecino de sí mismo. Eso sale `OK` —hash real, capture existente— y `check` no tiene con qué notar que lo que vigila es el propio fragmento. Un vecindario que se resuelve a sí mismo no falla: cubre, y afirma la cobertura que el nivel 1 existe para dar.

Así que la regla es un recorrido y no una proyección: se bajan los campos y se juntan los identificadores de tipo que haya adentro, a cualquier profundidad. `Result<Checked>` da dos, `(String, PathBuf, bool)` da tres, `(a: Foo, b: Bar)` da dos —el paréntesis no es ninguno, y los nombres de los parámetros tampoco—, y `void` da cero.

Y con eso el vecino que es el propio fragmento deja de poder escribirse, en vez de quedar prohibido por una guarda: preguntando sobre un identificador de tipo, no hay pregunta que devuelva la función que lo contiene.

### Los primitivos quedan afuera sin excluirlos

Una gramática le da kind propio a los tipos que no son un identificador —`primitive_type` en rust, `integral_type` y `void_type` en java, `predefined_type` en typescript—, así que juntar identificadores de tipo ya los deja fuera. Un `int` no tiene declaración a la que ir, y preguntar por él sólo puede devolver nada o el JDK.

Qué kinds nombran un tipo es de la gramática, igual que los campos de la firma, y vive del mismo lado por el mismo motivo: es la otra mitad de *"dónde hay un tipo que preguntar"*. La lista es por lenguaje, como todo lo demás en la gramática.

### El fold, y por qué el orden es por identidad

Un solo orden, y dos folds sobre ese orden:

```
orden = los vecinos por su id de capture, byte-wise. Es el orden en que se escriben
        en `n.1.link`, así que la clave de orden ya está en el archivo.

n.1.hash     = H( hash(v)     de cada vecino, en ese orden )
n.1.hash_ast = H( hash_ast(v) de cada vecino, en ese orden )
```

La clave de orden tiene que ser identidad, nunca contenido. Ordenando por el texto, un reformateo le cambiaría el puesto a un vecino, la lista se reordenaría, y el `hash_ast` del nivel se movería sin que ningún AST cambiara: un falso *"cambió de verdad"* producido por el orden.

El id de un capture es su identidad: `sha256(file \0 query \0)`. No lleva contenido, así que un reformateo no lo mueve; y cambia exactamente cuando el vecino entra, sale, se muda de archivo o se renombra, que son cambios de contrato.

Los vecinos se hashean con el mismo recorte de bordes que un fragmento ([capture.md](capture.md), "El rango excluye el espacio que rodea al nodo"): un vecino sin recortar mueve su hash cuando le agregan algo abajo.

### El `hash_ast` de un nivel es todo-o-nada

Si un vecino no tiene gramática no tiene `hash_ast`, y no puede quedar afuera del fold: un cambio real en ese vecino movería el `hash` del nivel y no su `hash_ast`, y eso se leería como *"sólo formateo"* cuando no lo fue. Un falso `RESTYLED` es peor que ningún estado.

Así que el campo está presente sólo si todos los vecinos tienen gramática. Si a alguno le falta, está ausente, y cualquier cambio en el `hash` del nivel es un cambio real.

`n` entero también es opcional: ausente donde el fragmento no tiene firma resoluble, que es prosa, un DTO, o un lenguaje sin anotaciones de tipo.

### Los vecinos no son captures acuñados

No se acuñan. Nadie los referencia con un `link` ni con un `accepted.link` propio, así que un capture acuñado para un vecino sería basura que `prune` borra en la pasada siguiente.

Son ubicaciones que alguien resuelve y que bilinker hashea al pasar. Con eso desaparecen la conversión rango → query, un modo nuevo de `capture`, y el modo de falla del anclaje: `capture` falla cuando no encuentra un ancla única, y con vecinos resueltos por un language server eso pasaría seguido.

### Bilinker no sale a buscarlos

Es un valor que bilinker guarda y compara sin poder calcularlo por su cuenta: el patrón de la copia opaca de un `accepted.link` de endpoint `path` o `repo`. Se compara, no se resuelve.

Quien los encuentra entra por un puerto que bilinker define y que no nombra a nadie:

```rust
pub trait Neighbours {
    fn of(&self, file: &Path, at: &[Position]) -> Result<Option<Vec<Location>>>;
}
```

Y lo que devuelve se vuelve un capture, no un hash. Cada ubicación resuelve a un nodo por la regla de siempre —la selección sirve para encontrar los nodos, no para recortarlos— y lo que queda escrito es un id por vecino, en `n.1.link`.

`None` es *"no pude mirar"* y no *"no hay vecinos"*: la distinción de la que sale el estado. El binario le pasa una implementación que le habla a `lspd`; la librería no lo menciona, para que bilinker no quede atado a ese daemon: mañana puede ser SCIP, un índice propio, o un language server hablado directo.

El puerto recibe posiciones, no el rango del fragmento. Dónde hay un tipo que preguntar es gramática, y la gramática es de bilinker; qué declara ese tipo es del proveedor. Pasarle el rango lo obligaría a inventar dónde preguntar adentro, y un proveedor que adivina eso está haciendo trabajo que no es suyo con conocimiento que no tiene.

Y el hasheo queda de este lado. El recorte de bordes es regla de bilinker y vive en el único lugar donde un nodo se convierte en rango. El proveedor devuelve ubicaciones, no hashes.

### Una lista vacía es una respuesta, y por eso el puerto no se puede defender solo

`Some(vec![])` dice *"miré y ninguna de estas posiciones resuelve a algo de esta capa"*, y es legítimo: `Result<Checked>` pregunta por los dos y `Result` vive en otro crate. Así que bilinker no puede tratar el vacío como sospechoso: plegarlo da el hash del string vacío y se escribe como vecindario adquirido, que es lo correcto para ese caso.

Lo que no puede es distinguirlo de *"el servidor de atrás todavía no indexó"*, porque llega igual. Y ahí el vacío se escribe afirmando una cobertura que no existe.

La distinción la tiene que dar quien la sabe. Un proveedor que no puede contestar tiene que decirlo contestando `None`, no un vacío. Bilinker no tiene con qué adivinarlo: si pusiera una guarda contra el vacío, rompería el caso legítimo, y si no la pone, come el ilegítimo. No hay una tercera opción de este lado del puerto: es el motivo de que el puerto tenga tres respuestas y no dos.

Del lado de `lspd` eso es `-32001`, que el binario traduce a `None`. Un proveedor que ni siquiera pueda saber si está listo devuelve lo que tenga: ahí la distinción no se puede dar en ningún lado, y eso es una propiedad del lenguaje, no un defecto que bilinker pueda tapar.

### Y un daemon de otro workspace es un `None`, no un fracaso

Medido el 2026-09-07: `bilinker check` en una capa de accreta falló con `file not found` sobre un archivo que existe, porque el daemon vivo era de otro proyecto. Un daemon ajeno no contesta *"no sé"*: contesta *"ese archivo no existe"*, y eso llegaba como un error del árbol. Es el mismo defecto de forma que el vacío —confundir *no pude mirar* con *la respuesta es no*— con la vuelta de que acá la respuesta falsa no es un vacío sino una negación, y sale por el canal de los errores.

| El daemon | |
|---|---|
| no está | `None` |
| es de otro workspace | `None`, y se dice cuál sirve |
| es de éste | se le pregunta |

Lo que se fija es que no se le cree al que no sirve, que es lo que hace honesto al tercer valor.

Con una puerta por workspace la pregunta se borra: desde que la ruta del socket de `lspd` se deriva del workspace, el que contesta en mi puerta es el mío por construcción. El chequeo se borra, y no porque se haya arreglado: porque el caso no se puede representar.

Y ahí sí bilinker levanta el suyo. Cómo lo levanta y cuánto lo espera son las dos reglas que siguen.

### El proveedor levanta el daemon que no contesta, una vez por corrida

Cuando la puerta del daemon de la capa no contesta, el proveedor que le habla a `lspd` lo levanta antes de contestar. Lo intenta una vez por corrida y no una por endpoint: `check` pregunta por decenas de endpoints, y cada uno no puede pagar su propio arranque.

Lo que no cambia es el tercer valor: si el daemon no arranca, sigue siendo `None`, y el aviso de puertas vivas que este cliente no calcula sale ahí, cuando igual degrada. Levantarlo es una comodidad, no una garantía, y un `check` que fracasara porque un daemon no arrancó estaría convirtiendo una falla de infraestructura en una reducción de cobertura.

Lo levanta quien le pregunta al puerto, y le pregunta sólo quien necesita tipos: `check`, `apply` y `accept`. `status`, `get` y el resto no preguntan, así que no levantan nada, y no hay una lista de comandos que mantener.

El daemon queda vivo cuando la corrida termina: el índice que construyó se reusa en la siguiente.

### Espera sólo lo que indexa el daemon que levantó, y Ctrl-C la corta

El daemon contesta `ping` antes de que sus language servers estén listos, los levanta a demanda con la primera pregunta de cada lenguaje, y mientras uno indexa contesta `-32001`. Levantar el daemon sin esperar no compraría nada en esa corrida: la primera pregunta volvería `-32001` y el vecindario degradaría igual.

Así que, si el proveedor levantó el daemon en esta corrida, un `-32001` no es `None` todavía. Consulta `status` cada segundo hasta que ningún servidor esté `INDEXING`, y vuelve a preguntar.

- **Espera todo lo que `status` da `INDEXING`**, y no "el servidor de este archivo": `status` nombra servidores, no lenguajes, y la tabla de qué servidor atiende qué extensión es del daemon. Con un daemon de esta corrida, lo que indexa lo arrancaron sus preguntas.
- **Un servidor `RUNNING` no se espera.** No informa readiness, así que no hay a qué esperar: se le pregunta, y un vacío suyo vale lo que vale para ese lenguaje.
- **Mientras espera, lo dice por stderr**, al empezar y cada quince segundos: qué servidores espera, en qué estado, cuánto va, y que Ctrl-C sigue sin vecindario.
- **Ctrl-C corta la espera y no la corrida.** El proveedor contesta `None`, el resto de la corrida sigue con el vecindario no verificado, y no vuelve a esperar. Fuera de una espera, Ctrl-C termina el proceso con 130, como siempre.
- **Si `status` deja de contestar, es `None`.** Y si no tiene nada `INDEXING` y la pregunta insiste en `-32001`, se vuelve a preguntar una vez —pudo quedar listo entre las dos— y la segunda es `None`.
- **No tiene techo de tiempo.** Lo que la corta es Ctrl-C.

Un daemon que ya estaba vivo cuando empezó la corrida es de otra: su `-32001` sigue siendo `None`, sin esperar. Es lo que evita que cada `check` corrido con un daemon vivo e indexando se vuelva una espera de minutos.

### Cuándo se adquiere el vecindario

El puerto puede contestar `None` —*"no pude mirar"*— y ahí hay que decidir qué se escribe. La regla es una: una falla de infraestructura no puede reducir la cobertura de un vínculo.

Que un fragmento tenga vecindario se sabe por la gramática y no por el proveedor. Y son tres respuestas, no dos:

| | Qué es | Qué se escribe |
|---|---|---|
| no hay | prosa, YAML, un lenguaje sin tipos, un DTO, un `enum`, una constante | la ausencia, sin marca y sin pedir nada |
| hay, y se alcanza | el fragmento es una firma, o está adentro de una | el `n` calculado sobre sus tipos |
| hay, y no se alcanza | el archivo entero, un `impl`, una clase con métodos | `accept` falla; `--no-n1` deja la renuncia escrita |

Lo que separa la primera de la tercera es si el fragmento contiene firmas que quedan sin cubrir. Un DTO no tiene ninguna adentro, así que su ausencia es completa y es la correcta. Un archivo entero de Rust tiene muchas y ninguna es la suya: eso no es *"no hay vecindario"*, es *"no puedo recorrer hacia los elementos del próximo nivel"*.

Un error que sale ahí tiene que decir por qué no se alcanza, porque quien lo lea no tiene cómo deducirlo:

```
Error: el fragmento de crates/bilinker/src/check.rs es el archivo entero, y su
       vecindario no se puede alcanzar: el nivel 1 sale de una firma, y un archivo
       tiene muchas — ninguna es la suya.
       Capturar el contrato con --as, o renunciar al vecindario con --no-n1.
```

De la segunda fila salen las posiciones que se le pasan al puerto: se sube hasta la firma que contiene al fragmento y se baja a los campos que llevan tipos —el retorno y los parámetros—, que son los mismos que [`--as interface`](chain.md) captura. Un capture de contrato y un capture de la función entera terminan preguntando en las mismas posiciones: el vecindario es de la firma, no de cómo se la haya capturado.

Y hay un dato más que se sabe sin daemon: el conjunto de vecinos lo determina la firma, y la firma está en el fragmento. Con el capture de contrato el `hash` del fragmento es el de la firma, así que un `hash` que no se movió es el mismo conjunto de vecinos, aunque su contenido no se haya podido mirar.

### El `n` previo tiene tres valores, y la tabla de qué se escribe

Puede estar adquirido, puede ser una renuncia escrita, o puede no estar, y los tres son entradas distintas, porque una renuncia es una decisión que alguien tomó y no la ausencia de una.

| `n` previo | Se pudo resolver | Cambió la firma | Qué se escribe |
|---|---|---|---|
| ausente | sí | — | `n` calculado |
| ausente | no | — | nada, y `accept` falla |
| `declined` | sí | — | `n` calculado: la renuncia se levanta sola |
| `declined` | no | — | la renuncia que ya estaba, intacta |
| adquirido | sí | — | `n` recalculado |
| adquirido | no | no | el `n` que ya estaba, intacto |
| adquirido | no | sí | nada, y `accept` falla |

Las dos filas de `declined` son la misma idea que las de adquirido: sin proveedor no hay información nueva con la cual revisar lo que ya se decidió, así que se conserva. Volver a pedirla convertiría la renuncia en algo que se tipea en cada `accept`, y un pedido que sale siempre no lo lee nadie.

Y conservarla no encierra a nadie: la tercera fila dice que con proveedor la renuncia se levanta sin que nadie pida nada. La asimetría es la correcta: subir cobertura es automático, bajarla sigue pidiendo que se declare.

La sexta fila es la que evita el daño. Preservar es estrictamente más seguro que borrar: si un vecino cambió mientras el proveedor estaba caído, el `n` viejo sigue ahí y el próximo cierre con proveedor lo reporta. Borrándolo, ese cambio se absorbe en el baseline nuevo y deja de ser detectable para siempre.

Y la séptima es la única donde preservar sería mentir: si la firma cambió, el conjunto de vecinos pudo cambiar con ella, y el valor viejo es sobre un conjunto que ya no es el de hoy. No tiene gemela en `declined` porque una renuncia no es sobre un conjunto: no dice qué vecinos había, dice que no se vigilan.

`--place` y `--content` ya aceptan una dimensión sin tocar la otra. El vecindario es una tercera y se comporta igual. Lo que no puede pasar es que una dimensión se borre como efecto colateral de aceptar otra.

### Y el vecindario tiene los mismos dos ejes que el fragmento

Con los vecinos siendo captures, el nivel 1 tiene dos ejes:

| eje | se compara | qué dice |
|---|---|---|
| ubicación | `n.1.link` contra `accepted[0].n.1.link` | un vecino entró, salió, se mudó de archivo o se renombró |
| contenido | el fold de hoy contra `accepted[0].n.1.hash` | la forma de un vecino cambió |

Es la misma división que arriba, un nivel más abajo, y con el mismo reparto de escritores: `apply` mantiene `n.1.link`, `accept` escribe la decisión.

Y son de verdad independientes: un nivel cuyo `link` es [`unknown`](bilink.md) conserva sus dos hashes y no tiene ids. Su eje de ubicación no se puede comparar, y no queda limpio: hay captures que alguien tiene que acuñar. Su eje de contenido se compara igual, contra el `hash` conservado. Por eso el contrato conservado no es un resto inservible: sin ubicación el nivel deja de detectar que un vecino se mudó o se renombró, y sigue detectando que la forma de un vecino cambió, que es lo que motivó al nivel 1. Cuál de los dos ejes nombra el estado es de [check.md](check.md).

Y por eso `apply` recibe el puerto. Un vecino cuyo archivo se renombró es un `MOVED` que git resuelve, pero el conjunto también gana y pierde miembros cuando la firma cambia, y qué tipo entró sólo lo sabe un language server. Todo comando que toque el eje del vecindario recibe el puerto, y degrada sin él. La frontera del subsistema no se mueve: la librería sigue siendo git y tree-sitter, y el proveedor entra por el puerto.

### `n: declined` es lo que vuelve determinista la renuncia

Renunciar al vecindario tiene que poder decirse, y las dos filas que fallan se destraban declarándolo con `--no-n1`. Eso escribe `n: declined`, y el campo no es cosmético: sin él se rompe la invariante 4.

Sin marca, el mismo fragmento en el mismo estado produce un `accepted` con `n` adquirido o sin él según si había un language server prendido en esa máquina. La determinación la tomaría el ambiente, que no es parte del estado del fragmento.

Con la marca, quien decide es el flag —igual que `--place` y `--content`— y la convergencia vuelve: dos personas que renuncian escriben lo mismo, y quien no renuncia no puede escribir un baseline mudo sin saberlo.

Y es lo único que cruza la frontera. El consumidor recibe una copia opaca del `accepted` del endpoint `repo` y no puede volver a mirar la gramática del fragmento ajeno para reconstruir el motivo. Sin la marca, *"el proveedor no tiene vecindario"* y *"el proveedor renunció a vigilarlo"* le llegan idénticos.

La ausencia sin marca queda con un solo significado: el fragmento no tiene firma resoluble. Eso es prosa, un DTO, o un lenguaje sin anotaciones de tipo, derivable de la gramática por cualquiera, siempre igual. Y por eso el caso *"hay y no se alcanza"* no puede escribirse como ausencia: escribiría *"no hay firma"* sobre un archivo lleno de firmas. Ese caso es una renuncia, y va con su marca como cualquier otra.

### Van en `accepted`, no en la cache

Dos razones, y las dos son las que decidieron dónde va cada campo del formato.

La frontera. Lo que cruza entre repos es la copia opaca de `accepted` del endpoint `repo`. Si `n` no está ahí, no cruza, y el caso que motiva todo esto se queda sin el mecanismo que lo entrega al consumidor.

No es recuperable. Reconstruirlo pediría un language server indexando un checkout histórico, que puede ni buildear. Ahí está la diferencia con `commit`, que se queda en la cache porque su recuperación es *"más lento, nunca no disponible"*. Un valor cuya reconstrucción depende de infraestructura que puede no estar no es un derivado: es una decisión.

Y pasa el otro test: converge. Dos personas que aceptan el mismo vecindario en el mismo estado escriben el mismo valor, porque sale de hashear contenido. Así que no le mete a `adopt` un campo que diverja siempre.

### Los cuatro cuadrantes

Dos ejes independientes, y las cuatro combinaciones dicen cosas distintas:

| Difiere | Qué significa |
|---|---|
| `hash` | el fragmento se reformateó |
| `hash` + `hash_ast` | el proveedor cambió lo que el fragmento dice |
| `n.1.hash` | el vecindario se reformateó |
| `n.1.hash` + `n.1.hash_ast` | un vecino cambió: el contrato se movió |

La última es el caso que motivó todo: el método intacto, el DTO movido.

## Quiénes aprobaron

### `agree` es el set de quienes aprobaron exactamente estos valores

Como los valores direccionan por contenido, *"estar de acuerdo"* no es ambiguo: es haber aprobado este hash, esta ubicación y este vecindario, y no otros.

La identidad de una entrada es su tupla entera: `link`, `hash`, `hash_ast` y `n`. Dos personas que aprueban el mismo fragmento con vecindarios distintos no comparten entrada: son dos contratos, y por lo tanto dos entradas ([bilink.md](bilink.md)). Y no hay endoso parcial de una entrada: con firma resoluble y sin proveedor `accept` se niega, así que *"aprobé la firma y el vecindario no lo miré"* no es un estado alcanzable.

Por endpoint y local, nunca copiado. En un endpoint estructural están los que aprobaron ese fragmento; en un endpoint `path` o `repo`, los que aprobaron esa copia. Quién aprobó del otro lado de la cadena es un hecho de la otra capa, y traerlo acá sería atribuir mal. Los dos endpoints de un bilink pueden tener listas distintas, y es lo normal.

Quien escribe el `accepted` entra solo. Si no, el campo significaría *"los que además aprobaron"*, que es otra cosa.

Es un set: ordenado y único al serializar. Un duplicado o un reordenamiento producirían un diff que no dice nada.

Y si cambia cualquiera de los cuatro, no se arrastra a nadie: se abre otra entrada. Quien aprobó los valores anteriores no aprobó los nuevos, y poner su nombre en la entrada nueva sería atribuirle una decisión que no tomó. La entrada nueva se suma y el endpoint queda `CONSENSUS_DIVERGED`: las dos decisiones están, visibles, y `check` falla hasta que alguien resuelva. Cuando alguien resuelve, la entrada que aprobaba otros valores se va, y donde queda es donde siempre quedó: en los commits que la escribieron. La lista no es un archivo histórico: es la ventana entre dos aceptaciones.

### Un nombre por línea, y por eso no guarda su commit

Se escribe en bloque, nunca en flow:

```yaml
agree:
- ana
- pablo
```

Porque `git blame` sólo puede atribuir una línea a un commit. En una sola línea, `- ana, - pablo` colapsa N actos distintos en un lugar, y blame devuelve el commit del último que la tocó: el primer aprobador se pierde. Con un nombre por línea, cada endoso queda atribuible por separado —autor, fecha y firma— y de ahí sale que el campo no necesite guardar el commit de nadie: git ya lo sabe, y con `blame` se llega en un salto.

Es la misma razón por la que `commit` no está en `accepted`: lo que git puede contestar no se duplica en el archivo.

Y el orden es alfabético, no cronológico. Cronológico diría quién aprobó primero, pero después de una unión el orden dependería del orden del merge, que no es un hecho sobre nada: dos repos con el mismo set escribirían archivos distintos. El alfabético es canónico, y no pierde nada: quién fue primero lo dice la fecha del commit, que blame entrega igual.

Insertar un nombre más arriba no rompe la atribución de los demás: blame sigue el contenido de la línea, no su número.

### Es lo que le da algo que escribir al segundo

Sin `agree`, aprobar algo que ya está `OK` no cambia ningún byte: no hay diff, no hay commit, no hay firma, no hay registro. El endoso explícito sería inexpresable. Con la lista, es un diff de una línea sobre un commit firmado.

Por eso el campo no duplica la historia: es lo que la crea. Parece derivable —caminar el log del bilink juntando autores— y sin él no habría log del cual derivarlo.

### Y sale de la convergencia byte a byte

Dos personas que aceptan el mismo contenido escriben los mismos hashes y listas distintas, así que `agree` sale de la fila *"ya coincidía"* de la que depende [`adopt`](ref.md).

La diferencia con un campo que no puede estar acá —`commit`, que el mismo contenido aceptado en dos ramas resuelve a dos valores sin forma de elegir— es que acá la resolución es correcta y única: unión. `adopt` es un merge campo por campo, así que une los dos sets sin preguntarle a nadie.

### Sin firma verificada es atribución, no atestación

`pablo` es texto, y cualquiera puede escribir `agree: [ana]` sin que Ana se entere. Un campo que parece atestación y es la afirmación de un tercero es peor que no tenerlo.

Vale lo que [ref.md](ref.md) dice: el autor de git es auto-declarado; lo que constituye atestación es la firma, no el campo.

Lo que la convierte en algo en que apoyarse son dos reglas que se verifican del lado que puede rechazar, y ninguna necesita traducir un nombre a una clave:

1. El commit está firmado por una clave de la allowlist, lo que lo ata al autor que declara.
2. Los nombres que ese commit agregó a algún `agree` son exactamente su autor.

Con las dos, `- ana` sólo puede haberlo escrito un commit firmado cuya autora es Ana. Las verifica `verify-ref`. Sin ellas —en un clon, o contra un remoto sin hook— `agree` se lee como lo que es: una declaración local, sin más peso que quien la escribió.

## Los valores son deterministas; la lista se une

### Aceptar es determinista

Dos personas que aceptan el mismo fragmento en el mismo estado escriben los mismos valores. `accepted` no lleva cuándo se aceptó ni desde qué HEAD: eso es del commit de la ref.

De ahí que `commit` sea el commit del contenido y no el HEAD de quien acepta. Con el HEAD, el mismo acto daba distinto según quién y cuándo lo hiciera, y el valor no describía nada del fragmento.

Lo único que no converge es `agree`, y a propósito: es el campo cuyo contenido es la diferencia entre las personas. Reconcilia por unión, que es la única regla posible y no requiere que nadie decida.

### `commit` es del contenido, no de quien acepta

`commit` es el commit en que el fragmento quedó con el contenido aprobado, y no el HEAD del repo al momento de aceptar.

Se calcula con un walk acotado hacia atrás resolviendo la query hasta que el hash cambie ([cache.md](cache.md), "Cómo se re-deriva"). Se calcula en `accept`, una vez, porque ahí está todo el contexto a mano; y se escribe en la cache, porque es derivable de `(accepted.link, accepted.hash)`.

Y la ventana de arqueología queda bien sin ajustes: `commit..HEAD` excluye `commit`, así que es exactamente *"lo que pasó después de que el contenido aprobado quedó establecido"*.

## Un endpoint `path` copia las dos

### La copia trae `link`, `hash` y `n`, y nunca `agree`

El `accepted` de un endpoint `path` o `repo` copia los valores del endpoint estructural del bilink adyacente: su `link`, su `hash` y su `n`. `agree` no se copia: es de esta capa, no del vecino.

Cada uno cambia por una sola razón y por ninguna otra: `hash` cuando cambia el contenido publicado, `link` cuando cambia su ubicación aprobada. Los dos son inmunes a etiquetas, comentarios y reordenamientos del archivo vecino, que es la razón de copiar los valores y no hashear el archivo entero.

## El comando `accept`

### `accept` toma un endpoint, un bilink entero, una capa o un path

```
bilinker accept <uuid>.<N>
bilinker accept <uuid>.<N> --place
bilinker accept <uuid>.<N> --content
bilinker accept <uuid>.<N> --no-n1
bilinker accept <uuid>.<N> --no-n1 --force
bilinker accept .
bilinker accept <path>
```

| Argumento | Descripción |
|-----------|-------------|
| `<uuid>.<N>` | Endpoint a aceptar: UUID del bilink + índice (0 o 1). |
| `--place` | Aprueba sólo la ubicación: escribe `accepted.link` y deja `accepted.hash` como estaba. |
| `--content` | Aprueba sólo el contenido: escribe `accepted.hash` y `accepted.hash_ast`. |
| `--no-n1` | Acepta renunciando al vecindario entero, del nivel 1 para arriba: escribe `n: declined` en vez de los niveles. |
| `--force` | Sólo junto a `--no-n1`, y sólo donde éste baja una cobertura que ya estaba. |
| `.` o `<path>` | Acepta en bulk todo lo que necesita atención en la capa actual (o bajo el path dado). |

Sin flags, aprueba las dos dimensiones.

### `accept` absorbe la rama, escribe `accepted` y cierra con un commit de un solo padre

1. Resolver el bilink y, para endpoints estructurales, el capture que su `link` referencia.
2. Si el capture no resuelve, fallar: no se puede aprobar contenido que no se pudo localizar.
3. Si el fragmento no está commiteado, fallar (ver "Exige el fragmento commiteado").
4. Si el tip de la rama del proyecto no está absorbido, absorberlo en un commit propio sobre [`refs/bilink/<branch>`](ref.md): un merge que sólo trae código, con el diff de `.bilink/` vacío. Es la misma forma que `sync`.
5. Calcular el hash del fragmento actual y su `hash_ast` si hay gramática.
6. Si el fragmento tiene firma resoluble, pedirle el vecindario al proveedor y resolver según la tabla de "El `n` previo tiene tres valores": calcularlo, preservar el que había, o fallar pidiendo `--no-n1`. Nunca borrarlo en silencio.
7. Escribir `accepted` en el endpoint: `link` con el id del capture vigente, `hash`, `hash_ast`, el `n` que corresponda —adquirido, `declined`, o ausente—, y `agree` con quien acepta agregado al set.
8. Calcular el `commit` del contenido y escribirlo en [la cache](cache.md).
9. Cerrar la aceptación con un commit sobre la ref, de un solo padre. Nunca un merge: sobre la ref un commit hace una cosa. Su mensaje es el comando canónico de esta aceptación —`accept [--place|--content] <uuid>.<N>`— y no lo que la persona tipeó, que va como trailer `Invocation:`.

El bilink cambia, y eso dispara `CHAIN_DIRTY` en el nodo adyacente en el próximo `check`. Es la única forma de mover una cadena: `check` no propaga nada porque no escribe ninguna decisión.

### Un commit por aceptación

Un `accept .` que aprueba veinte endpoints absorbe una vez —el paso 4— y escribe veinte commits encadenados sobre ese merge. La granularidad sigue al objeto y no a la invocación ([ref.md](ref.md), "Granularidad: un commit por decisión").

Absorber no es un paso de `accept`, es una precondición de escribir sobre la ref. Cuando el proyecto no se movió desde la última absorción no se absorbe nada; cuando sí, la absorción va en un commit propio inmediatamente antes.

Y en un repo que todavía no cortó a la ref no hay commit: los bilinks viven en la rama del proyecto, y commitearlos es de quien trabaja ([ref.md](ref.md), "Antes del corte no hay ref").

### Aceptar algo que ya está `OK` suma un nombre

Un endpoint en `OK` no tiene valores que cambiar. Con `agree`, un `accept` sobre él agrega a quien acepta al set, y eso es un diff, un commit y una firma. Es lo que vuelve expresable el endoso de un segundo revisor, y no cambia el estado: sigue en `OK`, con un aprobador más.

Si quien acepta ya está en el set, no hay nada que agregar y no se escribe ningún commit: publicar dos veces la misma aprobación no dice nada nuevo.

En bulk no entra. `accept .` toma *"todo lo que necesita atención"*, y un endpoint en `OK` no la necesita: sumar el nombre en veinte endpoints que nadie miró es la aprobación a ciegas. El endoso es por endpoint, nombrándolo.

### Quién es "quien acepta" lo dice git, no bilinker

El nombre que git usaría como autor, que es lo que `git var GIT_AUTHOR_IDENT` contesta. Que sea el mismo que el autor del commit y el mismo que `git blame` muestra sobre la línea del nombre es lo que permite cruzarlos: un `agree` que dijera una cosa y el autor del commit otra no se podría verificar contra ninguna firma.

Y por eso se pregunta así y no leyendo `user.name`. El nombre del autor no siempre sale de ahí: puede venir de `GIT_AUTHOR_NAME`, de un `[includeIf]` por directorio, o del sistema cuando nadie lo configuró. Leer un solo lugar acierta a veces, y cuando falla escribe en `agree` un nombre distinto del que va a quedar en el commit. Si git no puede contestar, `accept` falla, igual que fallaría el commit que viene después.

### `--no-n1`

No hay proveedor de vecindario y el fragmento tiene firma resoluble: `accept` no escribe.

```
$ bilinker accept a6d8b710.0
error: no hay proveedor de vecindario, y la firma de fetchPermissionsFromToken lo tiene.
       Aceptar así deja los tipos que la firma menciona sin vigilar, y el baseline no lo diría.
       Levantar lspd, o aceptar sin el nivel 1 con --no-n1.
```

Avisar y seguir no alcanza. Un warning por stderr que escribe igual es una línea más de texto: en un CI nadie lo lee, y el baseline mudo queda escrito. El aviso vale porque es la negativa.

Y el aviso es preciso o es ruido. Sólo aparece donde el fragmento tendría vecindario, que se sabe por la gramática, no por el proveedor. Aceptar prosa, un DTO o un lenguaje sin anotaciones de tipo no dice nada, porque ahí la ausencia de `n` ya era la correcta.

Y renuncia al vecindario entero, no al nivel 1. El día que exista un nivel 2 queda adentro de esta misma renuncia, porque está definido a través del 1. No va a haber un `--no-n2`: el flag nombra dónde empieza lo que necesita un language server, y no un escalón suelto.

### El `--force` está escalonado

`--no-n1` no alcanza donde el endpoint ya tiene `n` adquirido y la firma cambió, que es la única fila donde renunciar baja algo:

```
$ bilinker accept a6d8b710.0 --no-n1
error: a6d8b710.0 ya tiene un vecindario aceptado, y la firma cambió.
       Renunciar acá lo pierde: el conjunto de vecinos pudo cambiar con la firma,
       y sin proveedor no hay con qué reemplazarlo.
       Levantar lspd, o bajarlo a propósito con --no-n1 --force.
```

`--no-n1` en una persona se tipea una vez; en un CI se escribe una vez y queda para siempre. Una máquina sin language server lo necesita legítimamente para el caso donde no se pierde nada, y con un solo flag esa línea de configuración sería una autorización permanente a bajar cobertura cada vez que alguien cambie una firma. Escalonarlo deja al CI andando para el caso benigno y lo hace fallar justo cuando alguien tiene que mirar.

Es la forma que tiene [`capture remove --force`](capture.md): un guard sobre algo que otros nombran, un mensaje que da primero la salida no destructiva y después el override, y un force que lo hace igual y dice qué costó. Acá la salida no destructiva es levantar el proveedor.

`--force` es de `--no-n1`, no de `accept`. Solo, es un error. Un `--force` que modificara el comando entero se comería cualquier guard que se agregue después, en silencio y sin que nadie lo pida.

### Exige el fragmento commiteado

`accept` falla sobre un archivo sucio:

```
$ bilinker accept 7f3d8e9a.0
error: crates/bilinker/src/check.rs tiene cambios sin commitear.
       Aceptar fija un contenido, y ese contenido tiene que existir en la historia.
```

No es una recomendación. `commit` es el commit en que el fragmento quedó con el contenido aprobado, y ese commit no existe si el fragmento no está commiteado. Sin él no hay `git show <commit>:<file>`, y sin eso `check` no puede recuperar el texto aceptado, que es lo que distingue `EXPANDED` y `REANCHORED` de un `ALTERED` genérico.

### Salida y cuándo usarlo

```
accepted: 7f3d8e9a.1
  link:   capture 67ba7217…
  hash:   479922a1…
  commit: d4e5f6a7…   (el contenido quedó así en este commit)

note: el nodo adyacente detectará CHAIN_DIRTY en el próximo check
```

Tras `check`, sobre `PENDING`, `ALTERED`, `RESTYLED` o `CHAIN_DIRTY`, cuando el cambio es coherente con la intención del bilink. Tras `apply`, siempre: `apply` repunta la ubicación y la deja en `RELOCATED`. Ningún fix cierra solo.

| Código | Condición |
|---|---|
| 0 | Aceptación registrada. |
| 1 | UUID no encontrado, endpoint inválido, capture sin resolver, fragmento sin commitear, o vecindario no resuelto sin `--no-n1`. |

### Lo que no se acepta a ciegas

Un `accept .` sobre una capa recién cambiada fabrica aprobaciones que nadie miró. Cada estado no-OK es un puntero al fragmento que hay que revisar, y el inventario de trabajo de un cambio es esa lista; vaciarla para dejar el árbol verde es tirar justamente lo que hacía falta.

`accept .` existe para el caso en que ya se revisó todo, no para el caso en que no se revisó nada.

## Invariantes

1. `accepted` está completo o ausente. Su ausencia es `PENDING`.
2. `accept` es el único que escribe `accepted`.
3. `accept` falla si el fragmento no está commiteado.
4. Aceptar el mismo fragmento en el mismo estado produce siempre los mismos valores de `accepted`. `agree` es la excepción, y reconcilia por unión.
5. `accepted.link` de un endpoint `path` o `repo` es una copia opaca del id ajeno: se compara, no se resuelve.
6. Aprobar una ubicación y aprobar un contenido son actos separables.
7. `accepted.agree` es un set, incluye a quien escribió el `accepted`, y es local: nunca se copia de un vecino.
8. `agree` no participa de ninguna comparación de estado ni de ningún hash. `OK` no depende de cuántos aprobaron.
9. Un `accept` que cambia algún valor deja `agree` con quien aceptó y nadie más; uno que no los cambia lo agrega al set que había.
10. Un commit sobre la ref sólo agrega a su propio autor a un `agree`. Sacar no está restringido: agregar es lo único que afirma algo sobre otra persona.
11. Ningún `accept` reduce la cobertura de un endpoint sin que alguien lo haya pedido. Que el proveedor de vecindario no conteste nunca borra un `n` adquirido: o se preserva, o `accept` falla.
12. Un `accepted` sin `n` afirma que el fragmento no tiene firma resoluble. La renuncia se escribe, no se omite.
