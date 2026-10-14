//! Direction inherited by interface containers. Document geometry is independent.
//! Keep egui's LTR behaviour unchanged; roots opt into RTL, and nested columns keep it.

use egui::{Align, InnerResponse, Layout, Ui, UiBuilder, vec2};

pub(crate) fn enabled(ui: &Ui) -> bool {
    crate::i18n::current().is_rtl() && ui.layout().prefer_right_to_left()
}

pub(crate) fn root<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if crate::i18n::current().is_rtl() { ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Max)), add).inner } else { add(ui) }
}

pub(crate) fn column<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if enabled(ui) { ui.scope_builder(UiBuilder::new().layout(Layout::top_down(Align::Max)), add) } else { ui.vertical(add) }
}

/// Native columns reset their child layouts to LTR. Keep equal widths and the
/// native height calculation, but order and align the children at the reading edge.
pub(crate) fn columns<R>(ui: &mut Ui, count: usize, add: impl FnOnce(&mut [Ui]) -> R) -> R {
    if !enabled(ui) {
        return ui.columns(count, add);
    }
    ui.columns(count, |columns| {
        for column in columns.iter_mut() {
            *column = column.new_child(UiBuilder::new().max_rect(column.max_rect()).layout(Layout::top_down(Align::Max)));
        }
        columns.reverse();
        add(columns)
    })
}

/// Plain interface text fields keep logical text and native editing behavior.
pub(crate) fn singleline(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    let edit = egui::TextEdit::singleline(text);
    if crate::i18n::current().is_rtl() { edit.horizontal_align(Align::Max) } else { edit }
}

/// A label/value pair in a two-column form. The label leads in reading order;
/// the field keeps its own editing rules and its accessible label in either order.
pub(crate) fn form_field(ui: &mut Ui, label: &str, field: egui::TextEdit<'_>) -> egui::Response {
    if crate::i18n::current().is_rtl() {
        // A native grid's first column reports its previous intrinsic width.
        // Unlike a label, TextEdit clamps to that width, so seed it with the
        // normal field width instead of letting it remain at the grid minimum.
        let response = ui.add(field.min_size(vec2(ui.spacing().text_edit_width, 0.0)));
        let label = ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(label)).inner;
        response.labelled_by(label.id)
    } else {
        let label = ui.label(label);
        ui.add(field).labelled_by(label.id)
    }
}

pub(crate) fn menu(response: &egui::Response) -> egui::Popup<'_> {
    let popup = egui::Popup::menu(response);
    if crate::i18n::current().is_rtl() { popup.layout(Layout::top_down(Align::Max)) } else { popup }
}

pub(crate) fn context_menu(response: &egui::Response) -> egui::Popup<'_> {
    let popup = egui::Popup::context_menu(response);
    if crate::i18n::current().is_rtl() { popup.layout(Layout::top_down(Align::Max)) } else { popup }
}

/// Keep native menu activation/hover handling while inheriting the interface direction.
pub(crate) fn menu_button<R>(ui: &mut Ui, text: impl Into<egui::WidgetText>, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<Option<R>> {
    let text = text.into();
    if !crate::i18n::current().is_rtl() {
        return ui.menu_button(text, add);
    }
    let (response, inner) = if egui::containers::menu::is_in_menu(ui) {
        egui::containers::menu::SubMenuButton::from_button(egui::Button::new(text).left_text("◀")).ui(ui, |ui| root(ui, add))
    } else {
        egui::containers::menu::MenuButton::new(text).ui(ui, |ui| root(ui, add))
    };
    InnerResponse::new(inner.map(|r| r.inner), response)
}

/// egui grids require an LTR parent. Keep their semantic columns intact and align the
/// resulting form at the reading edge. Cache its width rather than running UI actions twice.
pub(crate) fn grid<R>(ui: &mut Ui, grid: egui::Grid, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if !enabled(ui) {
        return grid.show(ui, add);
    }
    let key = ui.next_auto_id().with("rtl_grid_width");
    let known = ui.data(|d| d.get_temp::<f32>(key));
    let width = known.unwrap_or_else(|| ui.available_width());
    let width = if width.is_finite() { width.max(0.0) } else { 0.0 };
    let response = ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| grid.show(ui, add));
    let actual = response.inner.response.rect.width();
    if actual.is_finite() && known.is_none_or(|old| (old - actual).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(key, actual));
        ui.ctx().request_repaint();
    }
    response.inner
}

pub(crate) fn checkbox(ui: &mut Ui, checked: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    if !crate::i18n::current().is_rtl() {
        return ui.checkbox(checked, text);
    }
    let text = text.into();
    let accessible = text.text().to_owned();
    let response = ui
        .allocate_ui_with_layout(vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = ui.spacing().icon_spacing;
            let mut mark = ui.checkbox(checked, "");
            let label = ui.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()));
            if label.clicked() {
                *checked = !*checked;
                mark.mark_changed();
            }
            mark.union(label)
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *checked, accessible.clone()));
    response
}

pub(crate) fn radio_value<T: PartialEq>(ui: &mut Ui, value: &mut T, alternative: T, text: impl Into<egui::WidgetText>) -> egui::Response {
    if !crate::i18n::current().is_rtl() {
        return ui.radio_value(value, alternative, text);
    }
    let text = text.into();
    let accessible = text.text().to_owned();
    let mut selected = *value == alternative;
    let response = ui
        .allocate_ui_with_layout(vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y), Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = ui.spacing().icon_spacing;
            let mut mark = ui.radio(*value == alternative, "");
            let label = ui.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()));
            if (mark.clicked() || label.clicked()) && *value != alternative {
                *value = alternative;
                selected = true;
                mark.mark_changed();
            }
            mark.union(label)
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected, accessible.clone()));
    response
}

pub(crate) fn radio(ui: &mut Ui, mut selected: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    if !crate::i18n::current().is_rtl() {
        return ui.radio(selected, text);
    }
    radio_value(ui, &mut selected, true, text)
}

/// Native combo interactions with direction-aware caption, arrow and popup contents.
pub(crate) struct ComboBox {
    inner: egui::ComboBox,
    selected: egui::WidgetText,
    width: Option<f32>,
}

impl ComboBox {
    pub(crate) fn from_id_salt(id: impl egui::AsIdSalt) -> Self {
        Self { inner: egui::ComboBox::from_id_salt(id), selected: "".into(), width: None }
    }
    pub(crate) fn selected_text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.selected = text.into();
        self
    }
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self.inner = self.inner.width(width);
        self
    }
    pub(crate) fn show_ui<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<Option<R>> {
        if !crate::i18n::current().is_rtl() {
            return self.inner.selected_text(self.selected).show_ui(ui, add);
        }
        let label = self.selected.text().to_owned();
        let pad = ui.spacing().button_padding;
        let icon = ui.spacing().icon_width;
        let gap = ui.spacing().icon_spacing;
        let width = self.width.unwrap_or(ui.spacing().combo_width).min(ui.available_width()).max(icon + gap + 2.0 * pad.x);
        let galley = self.selected.into_galley(ui, Some(egui::TextWrapMode::Truncate), width - icon - gap - 2.0 * pad.x, egui::TextStyle::Button);
        let response = self.inner.width(width).selected_text("").icon(|_, _, _, _| {}).show_ui(ui, |ui| root(ui, add));
        let rect = response.response.rect;
        let visuals = ui.style().interact(&response.response);
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let text_pos = egui::pos2(rect.max.x - pad.x - galley.size().x, rect.center().y - galley.size().y / 2.0);
        painter.galley(text_pos, galley, visuals.text_color());
        let center = egui::pos2(rect.min.x + pad.x + icon / 2.0, rect.center().y);
        crate::icons::paint(&painter, egui::Rect::from_center_size(center, vec2(icon, icon)), "dropdown", visuals.text_color(), visuals.text_color());
        response.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), label.clone()));
        response
    }
}
