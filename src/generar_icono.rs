//! Genera los iconos de la aplicacion a partir del SVG de la ventana
//! principal: el de Windows (recursos/raptor.ico, que build.rs embebe en el
//! ejecutable) y el de macOS (recursos/macos/raptor.icns, que va en el
//! paquete .app). Los dos quedan en el repositorio; solo hace falta
//! regenerarlos si cambia el dibujo.
//!
//! ```text
//! cargo test generar_icono -- --ignored
//! ```

use std::path::PathBuf;

use resvg::{tiny_skia, usvg};

// Tamanos que usa Windows: barra de tareas, Explorador, Alt+Tab y accesos
// directos, en las escalas del 100 % al 300 %.
const TAMANOS: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

/// Dibuja el SVG en un lienzo cuadrado del tamano pedido.
fn dibujar(arbol: &usvg::Tree, lado: u32) -> tiny_skia::Pixmap {
    let mut lienzo = tiny_skia::Pixmap::new(lado, lado).expect("tamano valido");
    let escala = lado as f32 / arbol.size().width().max(arbol.size().height());
    resvg::render(arbol, tiny_skia::Transform::from_scale(escala, escala), &mut lienzo.as_mut());
    lienzo
}

/// Imagen de un tamano para el .ico: PNG para 256 y mapa de bits de 32 bits
/// (BGRA, de abajo hacia arriba, con la mascara AND vacia) para los demas,
/// que es lo que leen todas las API de Windows, GDI+ incluida.
fn imagen_del_ico(lienzo: &tiny_skia::Pixmap) -> Vec<u8> {
    let lado = lienzo.width();
    if lado >= 256 {
        return lienzo.encode_png().expect("PNG del icono");
    }
    let bytes_de_mascara_por_fila = lado.div_ceil(32) * 4;
    let mut bmp = Vec::new();
    for valor in [40u32, lado, lado * 2] {
        bmp.extend_from_slice(&valor.to_le_bytes());
    }
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    let tamano_de_imagen = lado * lado * 4 + bytes_de_mascara_por_fila * lado;
    for valor in [0u32, tamano_de_imagen, 0, 0, 0, 0] {
        bmp.extend_from_slice(&valor.to_le_bytes());
    }
    let pixeles = lienzo.pixels();
    for fila in (0..lado).rev() {
        for columna in 0..lado {
            let color = pixeles[(fila * lado + columna) as usize].demultiply();
            bmp.extend_from_slice(&[color.blue(), color.green(), color.red(), color.alpha()]);
        }
    }
    bmp.resize(bmp.len() + (bytes_de_mascara_por_fila * lado) as usize, 0);
    bmp
}

/// Arma un .ico con una imagen por tamano.
fn ico_de(imagenes: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(imagenes.len() as u16).to_le_bytes());
    let mut desplazamiento = 6 + 16 * imagenes.len() as u32;
    for (lado, imagen) in imagenes {
        // 256 se escribe como 0 en el byte del ancho y del alto.
        let lado_en_byte = if *lado >= 256 { 0 } else { *lado as u8 };
        ico.extend_from_slice(&[lado_en_byte, lado_en_byte, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(imagen.len() as u32).to_le_bytes());
        ico.extend_from_slice(&desplazamiento.to_le_bytes());
        desplazamiento += imagen.len() as u32;
    }
    for (_, imagen) in imagenes {
        ico.extend_from_slice(imagen);
    }
    ico
}

/// Entradas del .icns de macOS: tipo y lado en pixeles, todas en PNG. Las
/// "@2x" son las de pantallas Retina (ic11 = 16 a doble densidad, etc.).
const ENTRADAS_DEL_ICNS: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

/// Dibuja el SVG dejando un margen alrededor, como piden las guias de Apple
/// para los iconos de aplicacion (el dibujo ocupa el 80 % del lienzo).
fn dibujar_con_margen(arbol: &usvg::Tree, lado: u32) -> tiny_skia::Pixmap {
    let mut lienzo = tiny_skia::Pixmap::new(lado, lado).expect("tamano valido");
    let contenido = lado as f32 * 0.8;
    let escala = contenido / arbol.size().width().max(arbol.size().height());
    let margen = (lado as f32 - contenido) / 2.0;
    let transformacion = tiny_skia::Transform::from_scale(escala, escala).post_translate(margen, margen);
    resvg::render(arbol, transformacion, &mut lienzo.as_mut());
    lienzo
}

/// Arma un .icns: "icns", el largo total y cada entrada con su tipo, su
/// largo (incluidos los 8 bytes de cabecera) y su PNG. Todo en big endian.
fn icns_de(entradas: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let largo_total: usize = 8 + entradas.iter().map(|(_, png)| 8 + png.len()).sum::<usize>();
    let mut icns = Vec::with_capacity(largo_total);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&(largo_total as u32).to_be_bytes());
    for (tipo, png) in entradas {
        icns.extend_from_slice(*tipo);
        icns.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        icns.extend_from_slice(png);
    }
    icns
}

#[test]
fn el_icns_tiene_su_cabecera_y_cada_entrada() {
    let icns = icns_de(&[(b"ic07", vec![1, 2, 3]), (b"ic08", vec![4])]);
    assert_eq!(&icns[..4], b"icns");
    assert_eq!(u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize, icns.len());
    assert_eq!(&icns[8..12], b"ic07");
    assert_eq!(u32::from_be_bytes(icns[12..16].try_into().unwrap()), 11);
    assert_eq!(&icns[19..23], b"ic08");
}

#[test]
#[ignore = "regenera recursos/macos/raptor.icns; se pide a mano"]
fn generar_icono_de_macos() {
    let raiz = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg = std::fs::read(raiz.join("ui/iconos/ventana-raptor.svg")).expect("SVG del icono");
    let arbol = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("SVG valido");
    let entradas: Vec<(&[u8; 4], Vec<u8>)> = ENTRADAS_DEL_ICNS
        .iter()
        .map(|&(tipo, lado)| (tipo, dibujar_con_margen(&arbol, lado).encode_png().expect("PNG del icono")))
        .collect();
    std::fs::create_dir_all(raiz.join("recursos/macos")).expect("carpeta de recursos de macOS");
    std::fs::write(raiz.join("recursos/macos/raptor.icns"), icns_de(&entradas)).expect("escribir raptor.icns");
}

#[test]
#[ignore = "regenera recursos/raptor.ico; se pide a mano"]
fn generar_icono_de_windows() {
    let raiz = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg = std::fs::read(raiz.join("ui/iconos/ventana-raptor.svg")).expect("SVG del icono");
    let arbol = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("SVG valido");
    let imagenes: Vec<(u32, Vec<u8>)> =
        TAMANOS.iter().map(|&lado| (lado, imagen_del_ico(&dibujar(&arbol, lado)))).collect();
    let ico = ico_de(&imagenes);
    assert_eq!(&ico[..4], &[0, 0, 1, 0]);
    std::fs::create_dir_all(raiz.join("recursos")).expect("carpeta de recursos");
    std::fs::write(raiz.join("recursos/raptor.ico"), ico).expect("escribir raptor.ico");
}
