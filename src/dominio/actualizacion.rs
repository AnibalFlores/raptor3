//! Actualizacion de los datos descriptivos del catalogo (titulo, artista,
//! album, anio y genero) desde una fuente externa: las etiquetas de los
//! archivos, releidas despues de corregirlas con otro programa (Picard, por
//! ejemplo), o internet (MusicBrainz, Deezer, iTunes). Se arma una lista de
//! cambios propuestos que el usuario revisa y elige antes de aplicarlos; lo
//! que corrigio a mano no se propone elegido.

use std::path::PathBuf;

/// Dato descriptivo de una ficha que puede venir de una fuente externa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CampoDescriptivo {
    Titulo,
    Artista,
    Album,
    Anio,
    Genero,
}

impl CampoDescriptivo {
    pub const TODOS: [Self; 5] = [Self::Titulo, Self::Artista, Self::Album, Self::Anio, Self::Genero];

    /// Clave fija (la del catalogo y la de la interfaz para el icono).
    pub fn clave(self) -> &'static str {
        match self {
            Self::Titulo => "titulo",
            Self::Artista => "artista",
            Self::Album => "album",
            Self::Anio => "anio",
            Self::Genero => "genero",
        }
    }

    pub fn desde_clave(clave: &str) -> Option<Self> {
        Self::TODOS.into_iter().find(|campo| campo.clave() == clave)
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// Campos que el usuario corrigio a mano (en el editor o por MCP) o eligio
/// de internet: al releer las etiquetas no se proponen elegidos. Las
/// fichas de versiones anteriores no lo registraban (`sin_registro`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CamposCorregidos {
    bits: u8,
    sin_registro: bool,
}

impl CamposCorregidos {
    /// Ficha de una version anterior: no se sabe que se corrigio.
    pub fn sin_registro() -> Self {
        Self { bits: 0, sin_registro: true }
    }

    pub fn desde_claves<'a>(claves: impl IntoIterator<Item = &'a str>) -> Self {
        let mut corregidos = Self::default();
        for campo in claves.into_iter().filter_map(CampoDescriptivo::desde_clave) {
            corregidos.marcar(campo);
        }
        corregidos
    }

    pub fn claves(self) -> Vec<&'static str> {
        CampoDescriptivo::TODOS.into_iter().filter(|campo| self.contiene(*campo)).map(CampoDescriptivo::clave).collect()
    }

    pub fn contiene(self, campo: CampoDescriptivo) -> bool {
        self.bits & campo.bit() != 0
    }

    /// Los mismos campos, en una ficha que no registra todas las correcciones.
    pub fn como_anterior(self) -> Self {
        Self { sin_registro: true, ..self }
    }

    pub fn es_anterior(self) -> bool {
        self.sin_registro
    }

    pub fn marcar(&mut self, campo: CampoDescriptivo) {
        self.bits |= campo.bit();
    }

    pub fn desmarcar(&mut self, campo: CampoDescriptivo) {
        self.bits &= !campo.bit();
    }
}

/// De donde vienen los datos propuestos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrigenDeLosDatos {
    /// Las etiquetas del archivo: lo aplicado deja de contar como corregido.
    Etiquetas,
    /// Internet (MusicBrainz, Deezer, iTunes): lo aplicado es una eleccion
    /// del usuario y cuenta como corregido (releer las etiquetas no lo
    /// propone pisar).
    Internet,
}

/// Valores descriptivos de un tema (los de la ficha o los leidos de una
/// fuente).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatosDescriptivos {
    pub titulo: Option<String>,
    pub artista: Option<String>,
    pub album: Option<String>,
    pub anio: Option<u16>,
    pub genero: Option<String>,
}

impl DatosDescriptivos {
    pub fn valor(&self, campo: CampoDescriptivo) -> Option<String> {
        let texto = match campo {
            CampoDescriptivo::Titulo => self.titulo.clone(),
            CampoDescriptivo::Artista => self.artista.clone(),
            CampoDescriptivo::Album => self.album.clone(),
            CampoDescriptivo::Anio => self.anio.map(|anio| anio.to_string()),
            CampoDescriptivo::Genero => self.genero.clone(),
        };
        texto.map(|texto| texto.trim().to_string()).filter(|texto| !texto.is_empty())
    }
}

/// Un cambio propuesto para un dato de un tema.
#[derive(Clone, Debug, PartialEq)]
pub struct Propuesta {
    pub archivo: PathBuf,
    pub campo: CampoDescriptivo,
    pub actual: Option<String>,
    pub nuevo: String,
    /// El usuario lo habia corregido a mano.
    pub corregido: bool,
    /// La ficha no registra que se corrigio (version anterior).
    pub sin_registro: bool,
    /// Elegida para aplicar.
    pub elegida: bool,
}

/// Cambios que propone una fuente para un tema: solo los datos que trae y
/// que son distintos de los de la ficha (nunca borra un dato).
///
/// Quedan elegidos de entrada:
/// - siempre, los que completan un dato vacio;
/// - de las etiquetas, los que reemplazan un dato que el usuario no corrigio
///   (si la ficha no lo registra, no se eligen: puede haberlo corregido);
/// - de internet, ningun reemplazo: el usuario decide.
pub fn proponer(
    archivo: PathBuf,
    actuales: &DatosDescriptivos,
    corregidos: CamposCorregidos,
    leidos: &DatosDescriptivos,
    origen: OrigenDeLosDatos,
) -> Vec<Propuesta> {
    CampoDescriptivo::TODOS
        .into_iter()
        .filter_map(|campo| {
            let nuevo = leidos.valor(campo)?;
            let actual = actuales.valor(campo);
            if actual.as_deref() == Some(nuevo.as_str()) {
                return None;
            }
            let corregido = corregidos.contiene(campo);
            let elegida = match (&actual, origen) {
                (None, _) => true,
                (Some(_), OrigenDeLosDatos::Etiquetas) => !corregido && !corregidos.es_anterior(),
                (Some(_), OrigenDeLosDatos::Internet) => false,
            };
            Some(Propuesta {
                archivo: archivo.clone(),
                campo,
                actual,
                nuevo,
                corregido,
                sin_registro: corregidos.es_anterior(),
                elegida,
            })
        })
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn datos(titulo: &str, artista: &str, anio: Option<u16>) -> DatosDescriptivos {
        let texto = |valor: &str| Some(valor.to_string()).filter(|valor| !valor.is_empty());
        DatosDescriptivos { titulo: texto(titulo), artista: texto(artista), anio, ..Default::default() }
    }

    #[test]
    fn los_campos_corregidos_se_guardan_por_clave() {
        let mut corregidos = CamposCorregidos::default();
        corregidos.marcar(CampoDescriptivo::Titulo);
        corregidos.marcar(CampoDescriptivo::Anio);
        assert_eq!(corregidos.claves(), ["titulo", "anio"]);
        assert_eq!(CamposCorregidos::desde_claves(["anio", "titulo", "otra"]), corregidos);
        corregidos.desmarcar(CampoDescriptivo::Titulo);
        assert!(!corregidos.contiene(CampoDescriptivo::Titulo) && corregidos.contiene(CampoDescriptivo::Anio));
        assert!(CamposCorregidos::sin_registro().es_anterior());
    }

    #[test]
    fn las_etiquetas_no_pisan_lo_corregido_a_mano() {
        let actuales = datos("amiga mia", "Sanz", None);
        let leidos = datos("Amiga mía", "Alejandro Sanz", Some(1997));
        let mut corregidos = CamposCorregidos::default();
        corregidos.marcar(CampoDescriptivo::Artista);
        let propuestas = proponer("a.mp3".into(), &actuales, corregidos, &leidos, OrigenDeLosDatos::Etiquetas);
        let elegida = |campo| propuestas.iter().find(|propuesta| propuesta.campo == campo).map(|propuesta| propuesta.elegida);
        assert_eq!(elegida(CampoDescriptivo::Titulo), Some(true));
        assert_eq!(elegida(CampoDescriptivo::Artista), Some(false), "corregido a mano");
        assert_eq!(elegida(CampoDescriptivo::Anio), Some(true), "completa un dato vacio");
        assert_eq!(elegida(CampoDescriptivo::Album), None, "la fuente no lo trae");
    }

    #[test]
    fn sin_registro_o_desde_internet_solo_se_completan_los_vacios() {
        let actuales = datos("Amiga mia", "", None);
        let leidos = datos("Amiga mía", "Alejandro Sanz", Some(1997));
        for (corregidos, origen) in [
            (CamposCorregidos::sin_registro(), OrigenDeLosDatos::Etiquetas),
            (CamposCorregidos::default(), OrigenDeLosDatos::Internet),
        ] {
            let propuestas = proponer("a.mp3".into(), &actuales, corregidos, &leidos, origen);
            let elegidas: Vec<_> = propuestas.iter().filter(|propuesta| propuesta.elegida).map(|propuesta| propuesta.campo).collect();
            assert_eq!(elegidas, [CampoDescriptivo::Artista, CampoDescriptivo::Anio], "{origen:?}");
            assert_eq!(propuestas.len(), 3);
        }
    }

    #[test]
    fn no_se_propone_lo_que_ya_es_igual_ni_borrar_un_dato() {
        let actuales = datos("Amiga mia", "Alejandro Sanz", Some(1997));
        let leidos = datos("Amiga mia", "", Some(1997));
        assert!(proponer("a.mp3".into(), &actuales, CamposCorregidos::default(), &leidos, OrigenDeLosDatos::Etiquetas).is_empty());
    }
}
