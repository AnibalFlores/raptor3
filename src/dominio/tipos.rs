//! Tipo de cada elemento del catalogo: decide como se trata en la lista,
//! en las tandas, en la botonera y en el registro de comerciales. El genero
//! queda libre para el estilo musical.

use super::duplicados::sin_acento;

/// Tipo de un elemento del catalogo. Un elemento sin tipo (sin clasificar)
/// se trata como un Tema.
///
/// - Tema: musica; se mezcla con el anterior y el siguiente.
/// - Tema editado: un tema con una locucion o edicion especial para la radio
///   (radio edit, intro locutada); se trata igual que un Tema.
/// - Comercial: propaganda facturable y calendarizada.
/// - Anuncio: mensaje propio de la emisora (no se factura).
/// - Publicidad: propaganda facturable pero no calendarizada.
/// - Efecto: corto de audio para pisar temas; solo puede estar en la botonera.
/// - Seco: voz en off; pisa como un efecto y ademas puede ir en la lista como
///   separador.
/// - Podcast: programa completo pregrabado; se puede pautar para que salga a
///   cierta hora (en la pauta de tandas), como un aviso calendarizado.
///
/// Todos menos el Tema y el Tema editado suenan sin mezcla, pueden repetirse en la lista y no
/// admiten efectos trabados ni efecto fijado. Las salidas al aire de lo
/// facturable (Comercial y Publicidad) se anotan en el registro de
/// comerciales.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TipoDeElemento {
    Tema,
    TemaEditado,
    Comercial,
    Anuncio,
    Publicidad,
    Podcast,
    Efecto,
    Seco,
}

impl TipoDeElemento {
    /// Todos los tipos, en el orden en que se ofrecen.
    pub const TODOS: [Self; 8] = [
        Self::Tema,
        Self::TemaEditado,
        Self::Comercial,
        Self::Anuncio,
        Self::Publicidad,
        Self::Podcast,
        Self::Efecto,
        Self::Seco,
    ];

    /// Los tipos que pueden ir en una tanda (el Podcast, para pautarlo a una
    /// hora).
    pub const DE_TANDA: [Self; 5] = [Self::Comercial, Self::Publicidad, Self::Anuncio, Self::Podcast, Self::Seco];

    /// Nombre guardado en el catalogo: no depende del idioma de la interfaz.

    pub fn nombre(self) -> &'static str {
        match self {
            Self::Tema => "Tema",
            Self::TemaEditado => "Tema editado",
            Self::Comercial => "Comercial",
            Self::Anuncio => "Anuncio",
            Self::Publicidad => "Publicidad",
            Self::Podcast => "Podcast",
            Self::Efecto => "Efecto",
            Self::Seco => "Seco",
        }
    }

    /// Nombre en el idioma de la interfaz.
    pub fn nombre_visible(self) -> String {
        match self {
            Self::Tema => tr!("Tema"),
            Self::TemaEditado => tr!("Tema editado"),
            Self::Comercial => tr!("Comercial"),
            Self::Anuncio => tr!("Anuncio"),
            Self::Publicidad => tr!("Publicidad"),
            Self::Podcast => tr!("Podcast"),
            Self::Efecto => tr!("Efecto"),
            Self::Seco => tr!("Seco"),
        }
    }

    /// Texto con el que se busca el tipo (`tipo:` / `type:`): el nombre del
    /// catalogo y el de la interfaz en inglés, asi la busqueda sirve en los
    /// dos idiomas. Los nombres en inglés son los del catalogo de traducciones.
    pub fn para_buscar(self) -> &'static str {
        match self {
            Self::Tema => "Tema / Song",
            Self::TemaEditado => "Tema editado / Edited song",
            Self::Comercial => "Comercial / Commercial",
            Self::Anuncio => "Anuncio / Announcement",
            Self::Publicidad => "Publicidad / Advertising",
            Self::Podcast => "Podcast",
            Self::Efecto => "Efecto / Effect",
            Self::Seco => "Seco / Dry voice",
        }
    }

    /// El tipo con ese nombre, sin distinguir mayusculas ni acentos.
    pub fn desde_nombre(nombre: &str) -> Option<Self> {
        let simplificado = simplificar(nombre.trim());
        Self::TODOS.into_iter().find(|tipo| simplificar(tipo.nombre()) == simplificado)
    }

    /// Posicion en `TODOS`.
    pub fn posicion(self) -> usize {
        Self::TODOS.iter().position(|tipo| *tipo == self).unwrap_or(0)
    }

    /// Musica que se mezcla con el anterior y el siguiente: Tema o Tema
    /// editado.
    pub fn es_musica(self) -> bool {
        matches!(self, Self::Tema | Self::TemaEditado)
    }

    /// Suena sin mezcla con el anterior y el siguiente y puede repetirse en
    /// la lista: todos menos el Tema y el Tema editado.
    pub fn sin_mezcla(self) -> bool {
        !self.es_musica()
    }

    /// Propaganda facturable (Comercial o Publicidad): sus salidas al aire se
    /// anotan en el registro de comerciales. Un Anuncio es un mensaje propio
    /// de la emisora y no se anota.
    pub fn facturable(self) -> bool {
        matches!(self, Self::Comercial | Self::Publicidad)
    }

    /// Un Efecto solo puede estar en la botonera, no en la lista.
    pub fn solo_para_la_botonera(self) -> bool {
        self == Self::Efecto
    }

    /// Puede ir en una tanda: Comercial, Publicidad, Anuncio, Podcast o Seco.
    pub fn va_en_tanda(self) -> bool {
        Self::DE_TANDA.contains(&self)
    }

    /// Efecto o Seco: los sonidos que el editor ofrece para fijar o probar
    /// sobre un tema.
    pub fn efecto_o_seco(self) -> bool {
        matches!(self, Self::Efecto | Self::Seco)
    }

    /// Franja de color de la fila en la lista: 0 ninguna (Tema o Tema
    /// editado), 1 comercial, 2 anuncio, 3 publicidad, 4 efecto, 5 seco y 7
    /// podcast (el 6 es el del anuncio HTH).
    pub fn franja(self) -> i32 {
        match self {
            Self::Tema | Self::TemaEditado => 0,
            Self::Comercial => 1,
            Self::Anuncio => 2,
            Self::Publicidad => 3,
            Self::Efecto => 4,
            Self::Seco => 5,
            Self::Podcast => 7,
        }
    }

    /// Tipo que corresponde a un genero de las versiones anteriores, que
    /// usaban el genero para esto. No distingue mayusculas ni acentos y
    /// acepta plurales o compuestos ("Anuncios", "Publicidad radial", "Tema
    /// seco"). None si es un genero comun: el elemento queda sin clasificar.
    /// Sirve para migrar el catalogo y para clasificar un archivo nuevo por
    /// el genero de sus etiquetas.
    pub fn segun_genero(genero: &str) -> Option<Self> {
        let simplificado = simplificar(genero);
        let palabras: Vec<&str> =
            simplificado.split(|letra: char| !letra.is_alphanumeric()).filter(|palabra| !palabra.is_empty()).collect();
        // Comercial, Anuncio y Publicidad se reconocen dentro de otras
        // palabras ("Comerciales"); Efecto y Seco solo como palabra completa
        // o su plural ("Secondhand" no es Seco).
        let dentro_de_otra = [Self::Comercial, Self::Anuncio, Self::Publicidad, Self::Podcast];
        let como_palabra = [Self::Efecto, Self::Seco];
        dentro_de_otra
            .into_iter()
            .find(|tipo| simplificado.contains(&simplificar(tipo.nombre())))
            .or_else(|| {
                como_palabra.into_iter().find(|tipo| {
                    let clave = simplificar(tipo.nombre());
                    palabras.iter().any(|palabra| *palabra == clave || palabra.strip_suffix('s') == Some(clave.as_str()))
                })
            })
    }
}

impl TipoDeElemento {
    /// Tipo que se deduce del genero al migrar el catalogo o al leer las
    /// etiquetas de un archivo nuevo: el que nombra el genero ("Publicidad
    /// radial") o, si es un genero comun ("Rock"), Tema. Sin genero no se
    /// deduce nada: queda sin clasificar.
    pub fn deducido_del_genero(genero: Option<&str>) -> Option<Self> {
        let genero = genero.map(str::trim).filter(|genero| !genero.is_empty())?;
        Some(Self::segun_genero(genero).unwrap_or(Self::Tema))
    }
}

fn simplificar(texto: &str) -> String {
    texto.chars().flat_map(char::to_lowercase).map(sin_acento).collect()
}

/// Reglas para un elemento que puede no tener tipo: sin clasificar se trata
/// como un Tema.
pub trait ReglasDelTipo {
    fn sin_mezcla(self) -> bool;
    fn facturable(self) -> bool;
    fn solo_para_la_botonera(self) -> bool;
    fn va_en_tanda(self) -> bool;
    fn efecto_o_seco(self) -> bool;
}

impl ReglasDelTipo for Option<TipoDeElemento> {
    fn sin_mezcla(self) -> bool {
        self.is_some_and(TipoDeElemento::sin_mezcla)
    }
    fn facturable(self) -> bool {
        self.is_some_and(TipoDeElemento::facturable)
    }
    fn solo_para_la_botonera(self) -> bool {
        self.is_some_and(TipoDeElemento::solo_para_la_botonera)
    }
    fn va_en_tanda(self) -> bool {
        self.is_some_and(TipoDeElemento::va_en_tanda)
    }
    fn efecto_o_seco(self) -> bool {
        self.is_some_and(TipoDeElemento::efecto_o_seco)
    }
}

#[cfg(test)]
mod pruebas {
    use super::TipoDeElemento::*;
    use super::*;

    #[test]
    fn el_tipo_se_busca_por_su_nombre_en_los_dos_idiomas() {
        for tipo in TipoDeElemento::TODOS {
            assert!(tipo.para_buscar().starts_with(tipo.nombre()), "{tipo:?}");
        }
        assert!(TipoDeElemento::TemaEditado.para_buscar().to_lowercase().contains("edited song"));
        assert!(TipoDeElemento::Seco.para_buscar().to_lowercase().contains("dry voice"));
    }

    #[test]
    fn deriva_el_tipo_de_los_generos_anteriores() {
        for (genero, tipo) in [
            ("Comercial", Comercial),
            ("COMERCIALES", Comercial),
            ("Anuncio", Anuncio),
            ("anuncios", Anuncio),
            ("Publicidad", Publicidad),
            ("Publicidad radial", Publicidad),
            ("Efecto", Efecto),
            ("Efectos de sonido", Efecto),
            ("Seco", Seco),
            ("Tema seco", Seco),
            ("Cortina (seco)", Seco),
            ("Podcasts", Podcast),
            ("Podcast de la manana", Podcast),
        ] {
            assert_eq!(TipoDeElemento::segun_genero(genero), Some(tipo), "{genero}");
        }
        for genero in ["Rock", "Pop", "", "Balada", "Secondhand rock", "Efectivo", "Dj Efectoman"] {
            assert_eq!(TipoDeElemento::segun_genero(genero), None, "{genero}");
        }
    }

    #[test]
    fn un_genero_comun_es_un_tema_y_sin_genero_no_se_clasifica() {
        assert_eq!(TipoDeElemento::deducido_del_genero(Some("Rock")), Some(Tema));
        assert_eq!(TipoDeElemento::deducido_del_genero(Some("Anuncios")), Some(Anuncio));
        assert_eq!(TipoDeElemento::deducido_del_genero(Some("  ")), None);
        assert_eq!(TipoDeElemento::deducido_del_genero(None), None);
    }

    #[test]
    fn cada_tipo_tiene_sus_reglas() {
        assert!(!Tema.sin_mezcla() && !TemaEditado.sin_mezcla());
        assert!([Comercial, Anuncio, Publicidad, Podcast, Efecto, Seco].iter().all(|tipo| tipo.sin_mezcla()));
        assert_eq!(TipoDeElemento::TODOS.into_iter().filter(|tipo| tipo.facturable()).collect::<Vec<_>>(), [Comercial, Publicidad]);
        assert_eq!(TipoDeElemento::TODOS.into_iter().filter(|tipo| tipo.va_en_tanda()).count(), 5);
        // El Podcast se pauta a una hora pero no se factura.
        assert!(Podcast.va_en_tanda() && !Podcast.facturable() && !TemaEditado.va_en_tanda());
        assert!(Efecto.solo_para_la_botonera() && !Efecto.va_en_tanda());
        assert!(Efecto.efecto_o_seco() && Seco.efecto_o_seco() && !Comercial.efecto_o_seco());
        assert_eq!((Tema.franja(), TemaEditado.franja(), Comercial.franja(), Seco.franja(), Podcast.franja()), (0, 0, 1, 5, 7));
        // Sin clasificar suena como un Tema.
        let sin_tipo: Option<TipoDeElemento> = None;
        assert!(!sin_tipo.sin_mezcla() && !sin_tipo.facturable() && !sin_tipo.va_en_tanda());
        assert!(Some(Publicidad).facturable());
    }

    #[test]
    fn reconoce_el_nombre_del_tipo() {
        assert_eq!(TipoDeElemento::desde_nombre(" publicidad "), Some(Publicidad));
        assert_eq!(TipoDeElemento::desde_nombre("TEMA"), Some(Tema));
        assert_eq!(TipoDeElemento::desde_nombre("Publicidad radial"), None);
        assert_eq!(TipoDeElemento::desde_nombre("tema EDITADO"), Some(TemaEditado));
        for tipo in TipoDeElemento::TODOS {
            assert_eq!(TipoDeElemento::desde_nombre(tipo.nombre()), Some(tipo));
            assert_eq!(TipoDeElemento::TODOS[tipo.posicion()], tipo);
        }
    }
}
