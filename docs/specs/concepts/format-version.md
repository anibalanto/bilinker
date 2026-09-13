# La versión del formato

El formato de los archivos de bilinker —los `.bilink` y los captures— es un crate, `bilink-format`, y la versión del crate es la versión del formato. No se puede cambiar el parseo sin releasear, ni releasear sin bumpear, y con eso el campo `.bilink/version` deja de ser una promesa y pasa a ser una propiedad del artefacto.

## El crate

### El formato vive en un crate propio, del que depende todo lo demás

```
bilink-format     los tipos y su serialización. No resuelve nada.
  └── bilinker    los interpreta: tree-sitter, git, estados
        ├── bilinker-cli
        └── bilinker-lsp
```

La línea que los separa es qué hace falta para leer un archivo y qué hace falta para juzgarlo. Un capture dice dónde está un fragmento; saber si el fragmento sigue ahí exige tree-sitter y git, y eso ya no es el formato. La mitad que describe el capture está en el crate de formato, y el algoritmo que lo produce —el walk-up por el AST, la construcción de la query— está en `bilinker`, porque depende de las gramáticas. El crate de formato no resuelve queries, no consulta git y no calcula estados.

### La versión del crate es la versión del formato

No hay otra copia del número: `.bilink/version` se escribe con la versión del crate, y cualquier comando la compara contra la del binario. Un `.bilink/` sin `version` es formato 1. La ausencia del directorio entero no es una capa, y no se lee como una versión.

## Se verifica, no se declara

### El hash del esquema se compara contra el registrado para la versión

Declarar la versión a mano no cubre el modo de falla que importa: un cambio aditivo que nadie bumpea. Agregar un tipo de endpoint no rompe ningún archivo existente, no deja entrada en el ledger de migraciones, y un parser viejo lo leería mal sin fallar: leería `abstract` como un path de capa y seguiría. Por eso hay un test:

```
sha256(esquema generado)  ==  <hash registrado para la versión N>
```

Cambiar los tipos sin subir la versión falla el test. La versión ordinal sirve para comparar y para leer; el hash garantiza que corresponde a lo que dice ser. Es el mismo principio que aplica a los captures: direccionar por contenido para que la identidad no dependa de que alguien se acuerde.

### El registro es de sólo-agregar

Una entrada registra lo que se publicó bajo esa versión. Corregirla en vez de agregar una nueva reescribiría el pasado, y el hash dejaría de certificar el artefacto que alguien ya descargó. El hash registrado para una versión es el sha256 del esquema publicado de esa versión, texto exacto.

### La regla protege lo publicado, y la línea es la publicación

Mientras una versión no salió —no hay release, ni tag, ni nadie que la haya descargado— su entrada todavía se está escribiendo, y corregirla no reescribe ningún pasado: no hay artefacto que dejar de certificar. La línea es la publicación, no el número.

Sin esa salvedad la regla pide algo que no puede cumplirse: subir el major para poder corregir un hash que nadie leyó, y con eso el número deja de significar lo que dice. Un major anuncia que un lector viejo no entiende un archivo nuevo; usarlo para arreglar el registro de una versión inédita lo convierte en un contador de correcciones.

Lo que la salvedad no hace es abrir la puerta a corregir en silencio: la entrada corregida lleva escrito al lado por qué se movió el esquema, porque el próximo que lea el registro va a encontrar un hash que no corresponde a ninguna release y necesita saber si eso fue deliberado.

### El esquema se puede mover sin que el formato cambie

Un doc comment se publica como `description`, así que corregir una frase mueve el hash del esquema sin tocar el formato. Es un tercer caso: no es que el formato cambió ni que se lo entiende mejor —el tipo es idéntico y lo que se entendía también—, es que el documento publicado lleva prosa adentro.

La salida no es sacar las descripciones del hash. El esquema se publica para que un consumidor valide sin adoptar bilinker, y la prosa es parte de lo que se publica: un artefacto cuyo hash no cubre la mitad que se lee no certifica el artefacto. Lo que distingue este caso de un cambio de formato es si salió, que es la misma línea de la regla anterior. Cómo verificar que fue sólo prosa es mecánico: generar el esquema de los dos lados y compararlos sin las descripciones. Si ahí no difieren, el formato no cambió.

### Sacar un campo sube el major

Quitar un campo no es aditivo: un lector de la versión nueva no entiende un archivo que lo lleve. `3.0.0` sacó el `offset` del capture, con lo que un fragmento dejó de ser un rango adentro de un nodo, y con él se fue `DISPLACED`, el único estado que hablaba de un sub-rango.

Los ids no cambian cuando se saca un campo. El id termina cada campo con un `\0` en vez de unirlos con separadores, así que el campo que desaparece contribuía la cadena vacía y su terminador sigue estando. Es una propiedad del formato del id, y vale para cualquier campo que se saque en el futuro.

### Un formato que ya no se escribe todavía se lee

`bilink-format-v1` está congelado en el sentido que importa: nadie escribe formato 1 nunca más. Eso no lo deja quieto: leerlo mejor también cambia el esquema, y el registro del crate viejo gana una entrada nueva con la anterior intacta. Pasó cuando el lector empezó a modelar `kind` y `name`, que estaban en el formato desde siempre y la migración no podía preservar porque nadie se los pasaba: el registro ganó `1.1.0` con `1.0.0` intacto.

La regla no distingue entre "el formato cambió" y "lo entendemos mejor": lo que registra es qué esquema se publicó bajo qué número, y los dos casos publican uno distinto. Un comentario que dijera "esta versión nunca va a tener otra entrada" sería una predicción, no una invariante.

### El esquema lleva su versión adentro

El documento publicado incluye el número de versión, así que el hash certifica las dos cosas a la vez: qué tipos describe y bajo qué nombre se publicó. Un esquema no puede circular diciendo ser una versión que no es.

## Qué tiene que aparecer en el esquema

### Lo que discrimina al parsear es visible en el esquema

Un esquema que describa de menos no sirve como guarda. Si el tipo de endpoint se publicara como `{"type": "string"}` a secas, agregar un tipo de endpoint no movería el hash, que es justo el cambio aditivo que motiva todo esto. Por eso los prefijos reconocidos se publican, y salen de la misma tabla que usa el parser: agregar un tipo obliga a tocar esa tabla, eso cambia el esquema, y el guard lo detecta.

Si el parser distingue por algo que el esquema no menciona, ese algo puede cambiar sin que nada se entere.

### Y a veces no se puede, y ahí sólo queda la versión

`3.3.0` es el caso: la `query` de un capture pasa a poder llevar varios `@target`, y el fragmento pasa a ser su concatenación. El tipo no cambió —`query` sigue siendo un string— y el archivo tampoco. Un parser de `3.2.0` lee una query de tres `@target`, se queda con el primero, y hashea otro fragmento: en silencio y sin fallar, que es el modo de falla que este registro existe para cubrir.

Y el esquema no puede describirlo: lo que discrimina está adentro de un string, y ningún tipo lo hace visible. Es el límite de la regla anterior. Donde el esquema no alcanza, subir la versión es lo único que le queda a un consumidor para saber que lo que lee no es lo que cree.

## Las dos direcciones

### El registro cubre una dirección, y la otra la cubre leer la versión

El registro protege una sola dirección: el parser viejo leyendo archivos nuevos. Contra eso hay tres cosas, y las tres son mecánicas: `deny_unknown_fields`, que lo hace fallar explícito en vez de ignorar un campo que no entiende; el hash del esquema, que hace imposible publicar dos formatos distintos con el mismo número; y que ningún slot tenga fallback.

La tercera es la misma regla que gobierna los prefijos. Un valor agregado a un slot cuyo parser rechaza lo que no reconoce falla explícito igual que un campo desconocido: `deny_unknown_fields` cubre los campos, y "un prefijo desconocido es un error, no un path" cubre los valores. Es lo que hace seguro agregar `unknown` al `link` de un nivel del vecindario: un parser de `4.0.0` no lo lee como un vecindario vacío, se niega. El caso opuesto es `3.3.0`, donde lo que cambió estaba adentro de un string que seguía siendo válido: ahí no había slot que rechazara nada, y sólo quedaba la versión.

El simétrico —parser nuevo, archivos viejos— no lo cubre ninguna de las dos, y no lo puede cubrir: los archivos viejos no tienen nada raro adentro, así que parsean bien. Un campo que se agregó con default se lee como su default, y un significado que cambió adentro de un string se lee con el significado nuevo. No hay nada en el archivo que delate de qué versión es. Lo único que lo dice es `.bilink/version`.

### Quien lee archivos de bilinker compara la versión antes de interpretarlos

`.bilink/version` es el único dato que discrimina en la dirección que el registro no cubre, así que un campo que se escribe y nadie compara es lo mismo que no tenerlo: un dato sin lector no se corrige nunca, porque no hay operación que lo obligue a estar bien. Con lector, el error aparece la primera vez que alguien corre un comando, que es cuando se puede arreglar.

Todo comando que interprete archivos de bilinker compara la versión declarada de la capa contra la del binario antes de interpretar, y se niega si el major difiere. No importa de quién sean los archivos: el que malinterpreta es el parser, y no le cambia nada que los archivos sean del repo de al lado o de éste. Vale para la capa propia igual que para la de un proveedor. Cómo lo hace `check` está en su spec, "Antes del primer bilink, la versión de la capa".

## El esquema publicado

### El esquema describe el archivo

Los `.bilink` y los captures son YAML serializado con serde desde los tipos del crate, así que el esquema JSON generado de esos tipos describe el archivo literal: qué campos hay, de qué tipo, cuáles son opcionales, y qué prefijos admite un `link`. Un tercero puede validar un archivo ajeno contra él.

### Para qué se publica

El esquema se genera desde los tipos y se publica como artefacto de la release:

```sh
cargo run -q -p bilink-format --bin schema > bilink-format-<version>.json
```

Un consumidor de la frontera lee los `.bilink` de otro proyecto. Con el esquema publicado los valida antes de interpretarlos, sin adoptar bilinker, con cualquier validador de JSON Schema en cualquier lenguaje. Eso baja el costo de adopción del lado del proveedor, que es lo que hay que minimizar. Un servidor que verifica una ref puede implementar las mismas filas desde el esquema, sin instalar el binario.

### La dirección es Rust → esquema

Los tipos con serde son la fuente y el esquema sale generado. La alternativa —esquema a mano, tipos generados— sacaría la definición del formato de Rust, que es su virtud, y agregaría codegen al build para un beneficio que hoy nadie cobra: ningún otro proyecto implementa su propio lector. Si aparece uno, es el momento de darlo vuelta.

## Invariantes

1. La versión del formato es la versión del crate `bilink-format`. No hay otra copia.
2. El hash registrado para una versión es el sha256 del esquema publicado de esa versión, texto exacto.
3. El registro de hashes es de sólo-agregar.
4. El esquema publica todo lo que el parser usa para discriminar.
5. El crate de formato no resuelve queries, no consulta git y no calcula estados.
6. Todo comando que interprete archivos de bilinker compara la versión declarada de la capa contra la del binario antes de interpretar, y se niega si el major difiere. Vale para la capa propia igual que para la de un proveedor.
7. Un `.bilink/` sin `version` es formato 1. La ausencia del directorio entero no es una capa, y no se lee como una versión.
