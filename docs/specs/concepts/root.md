# La raíz

Dónde empieza la capa sobre la que bilinker opera, qué es por clon y qué declara el proyecto. No hay archivo de configuración de la herramienta: todo lo que bilinker necesita lo descubre caminando el filesystem, y lo único que pide es que el clon esté puesto a punto.

## Dónde empieza una capa

### La raíz se resuelve caminando hacia arriba, hasta `.bilink/` o `.git/`

bilinker determina la raíz de la capa buscando desde el directorio de trabajo hacia arriba. Se detiene en el primer directorio que contenga alguno de estos marcadores, en este orden:

1. `.bilink/`, el directorio de bilinks de la capa, que es el marcador primario.
2. `.git/`, la raíz de un repositorio git, que es el marcador secundario.

Si ninguno se encuentra, la raíz es el directorio de trabajo actual. Es lo que permite usar bilinker en un proyecto nuevo sin ningún paso previo. La raíz se resuelve una vez por invocación del CLI, y todos los paths de los captures son relativos a la raíz de su capa.

Un `.bilink/` fabrica una raíz por ser marcador: si queda en un directorio que stratum no declara como capa, las dos herramientas discrepan sobre dónde termina una. Es el caso que `relayer` arregla, en la spec de la ref.

### No existe ningún archivo de configuración de la herramienta

Dentro de `.bilink/` hay tres archivos que bilinker escribe y lee —`version`, que dice qué formato son estos archivos; `cache/state`; y `head`, que dice de qué commit de la ref salió este árbol— y ninguno es configuración: nadie los edita a mano y todos salen de un comando. No existe `.bilinker.toml` ni ningún otro archivo que configure la herramienta, y ningún archivo de bilinker se escribe a mano.

## Lo que sí es por clon

### Dos líneas en `.git/`, y las pone `init`

| Dónde | Qué | Por qué ahí |
|---|---|---|
| `.git/info/exclude` | `.bilink/` y `.bilink-migrate-*` | `.gitignore` está versionado, y agregarlo modificaría la rama del proyecto |
| `.git/config` | el refspec de `refs/bilink/*` | sin él, `git fetch` trae la rama al día y deja los bilinks viejos |

No son configuración de bilinker sino del repo de quien lo usa, y por eso se piden en vez de escribirse solas: bilinker arregla solo lo que es suyo, y pide lo que es del repo del usuario. Sin `init` ningún comando corre. Que sean por clon y no por repo es lo que las obliga a estar ahí: no viajan con un `git clone`, así que lo primero que hace quien clona es correr `init`. Lo único que bilinker escribe fuera de `.bilink/` son esas dos líneas, y ninguna rama del proyecto cambia.

### Que el clon esté puesto a punto se detecta pidiendo las dos

Cada una cubre un caso que la otra no. El refspec es la que no puede estar por accidente —un `.bilink/` en el árbol puede venir de antes del corte, y el exclude lo pudo escribir alguien a mano—, pero no existe en un repo sin remoto, y ahí pedirla sola dejaría al repo sin forma de estar nunca inicializado: todo comando se negaría para siempre. Un repo local sin origen usa la ref igual, sólo que nunca la empuja.

## Lo que el proyecto sí declara

### `.bilink/.{alias}.toml` es una declaración del proyecto, no configuración

La frontera trae un archivo por proveedor:

```toml
# .bilink/.hsi.toml
remote = "git@gitlab…:minsal/hsi.git"
branch = "rc-2.32"
```

Es una declaración del proyecto sobre de qué depende, que es contenido, igual que la declaración de una subcapa de stratum. Que exista es lo que permite que ningún `.bilink` contenga una URL: toda la identidad del proveedor queda concentrada en un archivo por proveedor, y no repartida en N bilinks. Lo que sigue siendo cierto es que la raíz se descubre caminando hacia arriba, el lenguaje se infiere de la extensión, y nadie configura la herramienta para que corra.

## Capas y repositorios

### bilinker siempre opera en el contexto de una sola capa

Cada capa puede ser un repositorio git independiente, y la raíz encontrada es la raíz de la capa actual. Funciona sin configuración adicional porque sólo se puede crear un bilink sobre un archivo que esté presente en el filesystem local, lo que implica que el repositorio de esa capa ya está clonado. Por lo tanto los endpoints estructurales de una capa siempre referencian archivos del repo de esa capa, `git log` corre siempre en la raíz correcta, y para verificar una cadena completa se corre `bilinker check` en cada capa por separado.

## Lenguaje de los archivos

### El lenguaje se determina por la extensión del archivo

La gramática tree-sitter de un archivo referenciado sale de su extensión, sin configuración. La tabla completa está en la spec del capture, "Lenguajes soportados"; una extensión sin gramática se trata como texto plano, y ahí no hay `hash_ast` ni `RESTYLED`.

## Invariantes

1. La raíz se resuelve una vez por invocación del CLI.
2. Todos los paths de archivos en endpoints estructurales son relativos a la raíz de su capa.
3. No se requiere configuración explícita del lenguaje: la extensión es suficiente.
4. No existe `.bilinker.toml` ni ningún otro archivo que configure la herramienta. Los `.bilink/.{alias}.toml` de la frontera declaran de qué depende el proyecto, que es otra cosa.
5. Ningún archivo de bilinker se escribe a mano: todos salen de un comando.
6. Lo único que bilinker escribe fuera de `.bilink/` son las dos líneas por clon de `init`, en `.git/`. Ninguna rama del proyecto cambia.
