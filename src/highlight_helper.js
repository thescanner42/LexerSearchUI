let currentDecorations = [];

export function highlight_ranges_js(editor, elems) {
    if (!editor) return;

    const newDecorations = elems.map(e => ({
        range: {
            startLineNumber: e.start_line,
            startColumn: e.start_col,
            endLineNumber: e.end_line,
            endColumn: e.end_col,
        },
        options: {
            inlineClassName: e.class_name,
            hoverMessage: e.text ? [{ value: e.text }] : undefined,
        }
    }));

    currentDecorations =
        editor.deltaDecorations(currentDecorations, newDecorations);
}
