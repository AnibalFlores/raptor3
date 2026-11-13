//! Genera el icono de la aplicacion para Windows (recursos/raptor.ico) a
//! partir del SVG de la ventana principal. El .ico queda en el repositorio y
//! build.rs lo embebe en el ejecutable; solo hace falta regenerarlo si cambia
//! el dibujo.
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
