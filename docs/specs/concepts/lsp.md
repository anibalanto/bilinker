# El language server

`bilinker-lsp` muestra, dentro del editor, los bilinks que tocan el archivo abierto. No verifica nada y no escribe nada: es una vista.

## Qué es

### Contesta por LSP lo mismo que `get`, sin lógica propia

`bilinker get` contesta la misma pregunta, y contestarla desde el editor por CLI significaría un proceso por consulta y una integración distinta por editor. Un language server la contesta por un protocolo que los editores ya hablan: la extensión que lo consume no sabe nada de bilinker más allá de arrancarlo.

Por eso no tiene comandos propios. Todo lo que hace sale de las mismas dos llamadas que usa el CLI, la búsqueda por archivo y `get`: si el servidor tuviera lógica propia habría dos respuestas posibles a la misma pregunta.

## Transporte y capacidades

### El transporte es stdio

JSON-RPC de LSP por stdio. No abre puertos ni sockets: lo arranca el editor y muere con él.

### Declara exactamente dos capacidades, `hover` y `codeLens`

| Capacidad | Qué muestra |
|---|---|
| `hover` | Los endpoints que cubren la posición: su uuid, su estado y el fragmento del otro extremo. |
| `codeLens` | Una lente por línea donde empieza al menos un endpoint, con cuántos hay. |

Ninguna más.

### `hover` muestra los endpoints que cubren la posición

Para la posición del cursor, los endpoints cuyo capture la cubre, con su uuid, el estado que la cache ya dice y el fragmento del otro extremo. Un estado ausente se muestra ausente.

### El fragmento del otro extremo lleva el lenguaje de su archivo

`hover` muestra el fragmento en un bloque de código cuyo lenguaje sale de la extensión, con el nombre que resaltan los editores: `.rs` es `rust`, `.java` es `java`, `.yaml` y `.yml` son `yaml`, `.md` es `markdown`, `.feature` es `gherkin`, `.ts` y `.tsx` son `typescript`, `.js` y `.jsx` son `javascript`, y `.py` es `python`. Una extensión que no está en la lista va sin lenguaje.

### `codeLens` emite una lente completa por línea, sin resolver

`codeLens` no resuelve: `resolve_provider` es falso, y la lente se emite completa. Resolver perezosamente serviría si construirla fuera caro, y no lo es: sale del mismo escaneo que ya se hizo.

### La lente emite el comando `bilinker.showBilinks`, que ejecuta el editor

La lente lleva el comando con la URI del archivo y los ids de esa línea. El servidor no ejecuta ese comando: lo implementa la extensión, porque abrir un panel es decisión del editor y no del protocolo. Lo que la extensión hace con él es de la spec de lattice, que es donde vive la extensión de VS Code.

## Raíz y estado

### La raíz se detecta por archivo, no por workspace

Cada URI se resuelve con la misma detección de raíz que el CLI —caminar hacia arriba buscando `.bilink/` o `.git/`— y no con el `rootUri` que manda el editor. Es lo que hace que funcione en un workspace con varias capas: un archivo de un impl y uno de la raíz se resuelven contra capas distintas en la misma sesión, sin configuración.

Un archivo que no cae bajo ninguna capa no produce hover ni lentes. No es un error: es un archivo sin bilinks.

### El servidor no guarda estado entre consultas

No cachea nada, y por eso no puede quedar desactualizado respecto del disco. Cada hover vuelve a escanear. Cuando el escaneo pese, el lugar donde arreglarlo es el índice, que es un derivado compartido con el CLI, y no una cache propia del servidor, que sería una segunda verdad que invalidar.

## Lo que no hace

### No corre `check`, no escribe y no ofrece acciones de código

- No corre `check`. Muestra lo que la cache ya dice; un estado ausente se muestra ausente. Verificar es una acción del usuario, no un efecto de abrir un archivo.
- No escribe. Ni bilinks, ni cache, ni índice.
- No ofrece acciones de código. Aceptar o repuntar son decisiones, y una decisión no se toma desde un menú contextual sin ver el diff.

## Código de salida

| Código | Condición |
|---|---|
| 0 | El editor cerró la conexión. |
| 1 | No pudo hablar LSP por stdio. |
