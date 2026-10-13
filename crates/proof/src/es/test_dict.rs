//! A tiny synthetic Spanish Hunspell dictionary for tests (clean-room; written for these tests).
//! Real dictionaries are installed on the system, never bundled.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY eaosrnidlctumpbgvyqhfzjxñáéíóúükw
MAP 7
MAP aá
MAP eé
MAP ií
MAP oó
MAP uúü
MAP nñ
MAP csz
REP 4
REP decicion decisión
REP acer hacer
REP aver haber
REP ez es
SFX A Y 2
SFX A ar ado ar
SFX A ar ando ar
SFX B Y 1
SFX B ar aba ar
PFX X Y 1
PFX X 0 des .
";

pub const DIC: &str = "75
México
árbol
canción
español
computadora
trabajo
presidente
escribir
niño
éxito
decisión
habéis
por
favor
hacer
haber
es
dijo
que
existen
mail
guarda
ropa
ex
vice
casa
bueno
cielo
tierra
hombre
mujer
vida
tiempo
día
año
cosa
mundo
país
gobierno
parte
lugar
caso
manos
ojo
palabra
nombre
obra
ciudad
estado
calle
amigo
verdad
cuenta
agua
noche
fuerza
corazón
sol
mesa
libro
puerta
carta
cantar/A
hablar/B
cubrir/X
bien
mal
más
sí
no
ya
él
ella
usted
";

pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
