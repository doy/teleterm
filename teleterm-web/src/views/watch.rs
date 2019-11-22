use crate::prelude::*;
use unicode_width::UnicodeWidthStr as _;

pub(crate) fn render(model: &crate::model::Model) -> Node<crate::Msg> {
    let screen = if let Some(screen) = model.screen() {
        screen
    } else {
        return seed::empty![];
    };
    let (rows, cols) = screen.size();

    let mut grid = vec![];
    for row_idx in 0..rows {
        let mut row = vec![];
        for col_idx in 0..cols {
            let cell = screen.cell(row_idx, col_idx).unwrap();
            let mut contents = cell.contents();
            if contents.trim().is_empty() || contents.width() == 0 {
                contents = "\u{00a0}".to_string();
            }
            row.push(seed::td![
                seed::attrs! { At::Class => "cell" },
                contents
            ])
        }
        grid.push(seed::tr![seed::attrs! { At::Class => "row" }, row]);
    }

    seed::table![seed::attrs! { At::Class => "grid" }, grid]
}
