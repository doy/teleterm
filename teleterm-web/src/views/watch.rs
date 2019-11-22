use crate::prelude::*;

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
            row.push(seed::div![cell.contents()])
        }
        grid.push(seed::div![row]);
    }

    seed::div![grid]
}
