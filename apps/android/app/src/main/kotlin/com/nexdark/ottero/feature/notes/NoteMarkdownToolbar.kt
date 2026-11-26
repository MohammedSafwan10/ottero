package com.nexdark.ottero.feature.notes

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.ui.theme.LocalOtteroPalette

/**
 * Compact Markdown formatting toolbar docked above the keyboard in [NoteEditorScreen].
 * Wraps the current selection or inserts formatting at the cursor.
 */
@Composable
fun NoteMarkdownToolbar(
    value: TextFieldValue,
    onValueChange: (TextFieldValue) -> Unit,
) {
    val palette = LocalOtteroPalette.current

    fun wrap(prefix: String, suffix: String = prefix) {
        val text = value.text
        val start = value.selection.min.coerceIn(0, text.length)
        val end = value.selection.max.coerceIn(0, text.length)
        val selected = text.substring(start, end)
        val replacement = "$prefix$selected$suffix"
        val updated = text.replaceRange(start, end, replacement)
        val cursor = if (selected.isEmpty()) start + prefix.length else start + replacement.length
        onValueChange(TextFieldValue(updated, TextRange(cursor)))
    }

    fun prefixLine(prefix: String) {
        val text = value.text
        val pos = value.selection.min.coerceIn(0, text.length)
        val lineStart = text.lastIndexOf('\n', (pos - 1).coerceAtLeast(0))
            .let { if (it == -1 || pos == 0) 0 else it + 1 }
        val updated = text.substring(0, lineStart) + prefix + text.substring(lineStart)
        onValueChange(TextFieldValue(updated, TextRange(pos + prefix.length)))
    }

    Surface(color = palette.recessed) {
        HorizontalDivider(color = palette.border)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 2.dp),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            FormatChip("H", "Heading") { prefixLine("# ") }
            FormatChip("B", "Bold") { wrap("**") }
            FormatChip("I", "Italic") { wrap("_") }
            FormatChip("☑", "Checklist") { prefixLine("- [ ] ") }
            FormatChip("•", "Bullet") { prefixLine("- ") }
            FormatChip("</>", "Code") { wrap("`") }
            FormatChip("[[ ]]", "Wiki link") { wrap("[[", "]]") }
        }
    }
}

@Composable
private fun FormatChip(
    label: String,
    contentDescription: String,
    onClick: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    TextButton(onClick = onClick) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            color = palette.ink,
        )
    }
}
