//! Favorite interface themes: a star beside each theme on the Options theme cards, and a
//! right-click menu on View › Interface Theme that switches straight to a starred one.

use egui::{Color32, Response, Sense, Shape, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::Appearance;

/// The themes that can be starred: the fixed ones (System isn't a theme of its own).
pub const STARRABLE: [Appearance; 2] = [Appearance::Light, Appearance::Dark];

/// Checks a `ui.themeFavorites` list: theme codes, each at most once.
pub fn validate(value: &Value) -> Result<Vec<String>, String> {
    let items = value.as_array().ok_or("`themes` must be a list of theme codes")?;
    let mut favorites: Vec<String> = Vec::new();
    for item in items {
        let code = item.as_str().ok_or("theme codes are strings")?;
        let Some(a) = STARRABLE.into_iter().find(|a| a.code() == code) else {
            return Err(format!("unknown theme {code:?}; use light or dark"));
        };
        if favorites.iter().any(|f| f == a.code()) {
            return Err(format!("{code:?} is listed twice"));
        }
        favorites.push(a.code().to_string());
    }
    Ok(favorites)
}

/// Saved favorites as read from `ui.json`: unknown or repeated entries are dropped rather than
/// discarding the other settings.
pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let value = <Value as serde::Deserialize>::deserialize(d)?;
    let mut favorites: Vec<String> = Vec::new();
    for item in value.as_array().into_iter().flatten() {
        if let Some(a) = item.as_str().and_then(|c| STARRABLE.into_iter().find(|a| a.code() == c))
            && !favorites.iter().any(|f| f == a.code())
        {
            favorites.push(a.code().to_string());
        }
    }
    Ok(favorites)
}

fn is_favorite(app: &WordApp, a: Appearance) -> bool {
    app.ui.favorite_themes.iter().any(|f| f == a.code())
}

/// A 22 px star button: outlined, filled when `favorite`, drawn only when `visible`.
fn star(ui: &mut Ui, favorite: bool, visible: bool, accent: Color32, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if visible {
        let color = if favorite { accent } else { ui.visuals().weak_text_color() };
        let c = rect.center();
        let points: Vec<_> = (0..10)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                c + vec2(angle.cos(), angle.sin()) * if i % 2 == 0 { 7.0 } else { 3.0 }
            })
            .collect();
        if favorite {
            for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(10) {
                ui.painter().add(Shape::convex_polygon(vec![c, *a, *b], color, Stroke::NONE));
            }
        } else {
            ui.painter().add(Shape::closed_line(points, Stroke::new(1.2, color)));
        }
    }
    response.on_hover_text(label)
}

/// The star after a theme on its picker card (shown while the row is hovered or the theme is a
/// favorite); a click adds or removes it.
pub fn star_button(ui: &mut Ui, app: &mut WordApp, a: Appearance, row_hovered: bool) {
    let favorite = is_favorite(app, a);
    let action = if favorite { tl!("Remove from favorites") } else { tl!("Add to favorites") };
    let label = format!("{action}: {}", tl!(a.label()));
    let accent = ui.visuals().selection.stroke.color;
    if star(ui, favorite, favorite || row_hovered, accent, &label).clicked() {
        let mut themes = app.ui.favorite_themes.clone();
        if favorite {
            themes.retain(|f| f != a.code());
        } else {
            themes.push(a.code().to_string());
        }
        let _ = app.run("ui.themeFavorites", json!({ "themes": themes }));
    }
}

/// The right-click menu on View › Interface Theme: the options page, then the starred themes.
/// The ribbon button opens its own menu on a left click under `id.with("popup")`, the id a plain
/// context menu would also use, so this one gets its own.
pub fn shortcut_menu(resp: &Response, app: &mut WordApp) {
    egui::Popup::context_menu(resp).id(resp.id.with("theme_favorites")).show(|ui| {
        ui.set_min_width(200.0);
        if ui.button(tl!("Appearance Settings…")).clicked() {
            app.ui.backstage = true;
            app.ui.backstage_page = "options".into();
            ui.close();
        }
        let favorites: Vec<Appearance> = STARRABLE.into_iter().filter(|a| is_favorite(app, *a)).collect();
        if !favorites.is_empty() {
            ui.separator();
        }
        for a in favorites {
            if ui.add(egui::Button::selectable(app.ui.theme == a, tl!(a.label()))).clicked() {
                let _ = app.run("ui.theme", json!({ "value": a.code() }));
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;
    use crate::{Services, UiState};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    #[test]
    fn favorites_are_checked_saved_and_read_back() {
        let mut a = app();
        assert_eq!(a.run("ui.themeFavorites", json!({"themes": ["dark", "light"]})).unwrap()["themes"], json!(["dark", "light"]));
        assert_eq!(a.run("ui.themeFavorites", json!({})).unwrap()["themes"], json!(["dark", "light"]), "no `themes` reads them");
        for junk in [json!(["system"]), json!(["purple"]), json!(["dark", "dark"]), json!([1]), json!("dark"), json!(null)] {
            assert!(a.run("ui.themeFavorites", json!({ "themes": junk })).is_err(), "{junk}");
        }
        assert_eq!(a.ui.favorite_themes, ["dark", "light"], "a rejected list changes nothing");
        let saved = serde_json::to_string(&a.ui).unwrap();
        let restored: UiState = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.favorite_themes, ["dark", "light"]);
        // A damaged list loses only its bad entries, never the other settings.
        let odd: UiState = serde_json::from_value(json!({"favoriteThemes": ["dark", "x", 3, "dark"], "tab": "Insert"})).unwrap();
        assert_eq!((odd.favorite_themes, odd.tab.as_str()), (vec!["dark".to_string()], "Insert"));
        let odd: UiState = serde_json::from_value(json!({"favoriteThemes": "dark", "tab": "Insert"})).unwrap();
        assert!(odd.favorite_themes.is_empty());
        assert!(UiState::default().favorite_themes.is_empty());
    }

    /// Star Dark on its theme card, then pick it from the right-click menu.
    #[test]
    fn star_a_theme_then_pick_it_from_the_shortcut_menu() {
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(640.0, 400.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                if !fonts {
                    crate::theme::install_fonts(ui.ctx());
                    fonts = true;
                    return;
                }
                let resp = ui.button("Interface Theme");
                shortcut_menu(&resp, app);
                crate::appearance::theme_picker(app, ui);
            },
            app(),
        );
        h.run();
        h.get_by_label("Add to favorites: Dark").click();
        h.run();
        assert_eq!(h.state().ui.favorite_themes, ["dark"]);
        assert!(h.query_by_label("Remove from favorites: Dark").is_some());
        assert!(h.query_by_label("Add to favorites: Light").is_some());

        h.get_by_label("Interface Theme").click_secondary();
        h.run();
        assert!(h.query_by_label("Appearance Settings…").is_some());
        assert_eq!(h.query_all_by_label("Light").count(), 1, "Light isn't starred, so only its card row shows");
        assert_eq!(h.query_all_by_label("Dark").count(), 2, "Dark's card row and its shortcut");
        h.query_all_by_label("Dark").last().unwrap().click();
        h.run();
        assert_eq!(h.state().ui.theme, Appearance::Dark);
        assert_eq!(h.state().ui.favorite_themes, ["dark"]);

        h.get_by_label("Interface Theme").click_secondary();
        h.run();
        h.get_by_label("Appearance Settings…").click();
        h.run();
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "options");
    }
}
