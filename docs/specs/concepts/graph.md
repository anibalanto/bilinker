# El grafo

`bilinker graph` exporta las aristas de los bilinks en el modelo de aristas de lattice. Es la forma en que bilinker actúa como proveedor de `lattice graph`: resolver una cadena a través de capas es conocimiento del formato bilink, y recorrer el grafo y componerlo con las aristas de otros proveedores es de lattice. No recorre ni modifica nada.

## El selector

### Un selector es un archivo, una posición, un UUID o toda la capa

```
bilinker graph <selector>
  [--format json]
  [--recursive]
```

| Selector | Qué bilinks exporta |
|----------|---------------------|
| `archivo.md` | Los que referencian ese archivo en la capa actual |
| `archivo.md:42:5` | Los mismos que el archivo: la posición no filtra |
| `<uuid>`, ocho o más caracteres hexadecimales | Un bilink concreto, por UUID o prefijo |
| `.` o `*` | Todos los bilinks de la capa actual, los `.yaml` de su `.bilink/`; con `--recursive`, los de todas las capas bajo la raíz del proyecto |

### Sin ninguna arista que emitir, sale con 1

Un selector que no encuentra ningún bilink sale con 1, y lo dice por stderr. También sale con 1 una capa con bilinks de la que no sale ninguna arista, como una capa sin `check` corrido, y el mensaje dice que hay que correrlo. Un error, como un UUID que no existe o un bilink que no se puede leer, también sale con 1. Para lattice cualquiera de los tres es un proveedor que no contestó, y no un grafo vacío.

### Con algún bilink sin rango en la cache, emite lo que tiene y sale con 3

Un bilink cuyo tip no tiene rango en la cache, como uno creado y aceptado después del último `check`, no emite arista. Si otros sí la emiten, `graph` las emite, dice por stderr cuántos bilinks quedaron afuera y que hay que correr `bilinker check .`, y sale con 3. Para lattice es un proveedor que contestó incompleto, y no un grafo completo.

## Los formatos

### `json` es el único formato, y el que se usa sin `--format`

`graph` no tiene `tree`, `flat` ni `--depth`: recorrer, limitar la profundidad y renderizar son de `lattice graph`, que muestra además las aristas de los otros proveedores. `--format` sigue existiendo porque es la línea con que lattice invoca al proveedor, y un valor que no es `json` es un error de uso.

### `json` es el contrato de proveedor hacia lattice

Emite las aristas de bilinker en el modelo de aristas de lattice, con los nodos ya resueltos a forma canónica.

```json
[
  {"from":".::commands/pull.md#312~358","to":".stratum/impl::crates/stratum-cli/src/main.rs#245~389","kind":"bilink","guarantee":"accepted","provider":"bilinker","directed":false,"ref":"c0feab23-1b2c-4d5e-8f6a-7b8c9d0e1f2a","state":["OK","OK"],"commit":["81acc9b","93b4582"]}
]
```

`state` lleva la tupla de estados de los dos tips, y `commit` el commit en que se aceptó cada uno, que es el baseline de `git log <commit>..HEAD`. `declaration` sale sólo cuando algún tip tiene una, con la del otro en `null`: `"declaration":[null,"106~262"]`. Un estado que la cache no tiene sale como `—`. Los `kind` emitidos son `bilink` e `issue`, los dos con garantía `accepted`. `governs` no se emite: exige el endpoint de tipo bilink, que está especificado y no implementado.

La capa de un nodo se nombra relativa a la raíz más externa del ecosistema que contiene a la capa invocada, así que el mismo fragmento tiene la misma forma canónica desde cualquier capa.

## La cadena

### Una cadena de N nodos emite una arista entre sus dos tips estructurales

No N-1 aristas entre nodos `.bilink`. Los mids son mecanismo interno de bilinker, no conexiones del proyecto. Una cadena que se encuentra desde dos selectores, o desde dos capas con `--recursive`, sale una sola vez.

### Los tips se buscan cruzando capas por los endpoints `path`

Desde el bilink de la capa, cada endpoint `path` lleva al bilink del mismo UUID en la capa vecina, y cada endpoint `capture` es un tip. Si la capa vecina no está clonada, la búsqueda se detiene ahí sin fallar.

Una cadena que no llega a dos tips estructurales no emite arista: una punta `abstract`, un endpoint `repo` —que es un fragmento de otro proyecto, y la búsqueda no cruza la frontera— o una capa vecina que no está clonada.

### El rango de un tip es el que dejó el último `check` en la cache

El rango vigente de un fragmento es derivado, y vive en la cache de su capa. Un tip sin rango en la cache no tiene forma canónica, y su cadena no emite arista: lattice necesita `check` corrido antes de consultar.

### Un rango de varias partes sale con un tramo por parte

Un capture de varias partes, como el de `spring-controller`, sale con un tramo `inicio~fin` por parte, en orden de archivo y separados por coma: `.::src/Service.java#16~51,106~144,156~180,195~209`. El texto entre dos partes no está en ningún tramo, porque el fragmento no lo cubre.

### Un tip de varias partes lleva la declaración que nombra su capture

La declaración es el nodo que declara el ancla del capture, el nombre de su último predicado `#eq?`: en un `spring-controller`, el método entero, con su cuerpo. Sale en `declaration`, un tramo por tip, o `null` en un tip de una sola parte.

Se resuelve con tree-sitter sobre el archivo de hoy, y sale sólo si ese mismo match da los tramos que tiene la cache: si no, el archivo cambió desde el último `check`, y una declaración de hoy junto a tramos viejos no nombra lo mismo. Un capture sin ancla, o cuya query ya no resuelve, sale sin declaración.

## Invariantes

1. `graph` nunca modifica ningún archivo.
2. Fragmentos distintos del mismo archivo generan nodos separados, cada uno con su rango.
