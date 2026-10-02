// Empêche l'ouverture d'une console supplémentaire sous Windows en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    classeur_urssaf_lib::run()
}
