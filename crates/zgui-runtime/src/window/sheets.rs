//! Style sheets a view installed for itself, applied to this window's rule set.
//!
//! An application's sheet is installed once, when the window opens. A component's sheet cannot be:
//! a component is a function anyone may call, it carries its own rules, and the program using it
//! never wrote them down. So a view asks, the ask is queued as a command, and this is where the
//! queue meets the style engine — after the reactive flush and before the restyle, so a component
//! that mounted this frame is styled by its own sheet in the frame it appeared in.

use zgui_style::{SheetOrigin, SheetSource};

use crate::window::Window;

impl Window {
    /// Installs a view's style sheet under `name`, or replaces it when the name is taken.
    ///
    /// Replacement keeps the sheet's place in the cascade. Removing and adding would move it to
    /// the end of the author origin, where it would begin winning against every sheet that used to
    /// beat it — so a theme that changed a colour would also, invisibly, change what wins.
    pub(crate) fn install_view_sheet(
        &mut self,
        name: &str,
        css: &str,
        base: Option<&zgui_css::resources::ResourceBase>,
    ) {
        let source = match base {
            Some(base) => SheetSource::Based(css, base),
            None => SheetSource::Text(css),
        };
        let document = self.document.borrow();
        let diagnostics = match self.view_sheets.get(name) {
            Some(handle) => {
                let diagnostics = self.engine.replace_sheet(&document, handle, source);
                // Replaced rules may have styled any element in the tree, and rules that are gone
                // leave no selector to invalidate by.
                self.engine.force_author_rules_dirty();
                diagnostics
            }
            None => {
                // An added sheet is appended to the rule set and invalidates by its own
                // selectors. The elements the component that installs it is building have no
                // style yet, so they are styled against it in this frame whatever it invalidates.
                let (handle, diagnostics) =
                    self.engine
                        .add_sheet(&document, SheetOrigin::Author, source);
                self.view_sheets.insert(name.to_owned(), handle);
                diagnostics
            }
        };
        drop(document);
        for report in diagnostics.iter() {
            tracing::warn!(
                target: "zgui::css",
                sheet = name,
                "{}",
                report.message
            );
        }
        self.load_font_faces();
    }

    /// Hands the `@font-face` rules of the sheets installed since the last call to the window's
    /// font loader, when it has one.
    pub(crate) fn load_font_faces(&mut self) {
        let faces = self.engine.take_font_faces();
        if faces.is_empty() {
            return;
        }
        match &self.font_faces {
            Some(loader) => loader.load(&faces),
            None => tracing::debug!(
                target: "zgui::css",
                faces = faces.len(),
                "a sheet declares faces and the window has no font loader"
            ),
        }
    }

    /// Removes the sheet installed under `name`. Removing one that is not installed does nothing.
    ///
    /// A sheet is removed by letting go of its handle: the rule set reclaims every sheet whose
    /// handle has been dropped, which is what makes "forgot to remove it" impossible to write.
    pub(crate) fn remove_view_sheet(&mut self, name: &str) {
        drop(self.view_sheets.remove(name));
    }
}
