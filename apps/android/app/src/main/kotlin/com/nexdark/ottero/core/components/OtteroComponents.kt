package com.nexdark.ottero.core.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.ui.theme.LocalOtteroPalette

val OtteroCardShape = RoundedCornerShape(14.dp)

@Composable
fun OtteroCard(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = OtteroCardShape,
        color = palette.card,
        border = BorderStroke(1.dp, palette.border),
        shadowElevation = if (palette.isDark) 0.dp else 1.dp,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            content = content,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OtteroTopChrome(
    title: String,
    showDrawerButton: Boolean,
    onOpenDrawer: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = palette.canvas,
            titleContentColor = palette.ink,
            navigationIconContentColor = palette.ink,
        ),
        navigationIcon = {
            if (showDrawerButton) {
                IconButton(onClick = onOpenDrawer) {
                    Icon(
                        imageVector = Icons.Outlined.Menu,
                        contentDescription = "Open menu",
                    )
                }
            }
        },
        title = {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )
        },
    )
}

@Composable
fun SectionHeader(
    title: String,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    val palette = LocalOtteroPalette.current
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            color = palette.ink,
        )
        if (actionLabel != null && onAction != null) {
            TextButton(onClick = onAction) {
                Text(
                    text = actionLabel,
                    style = MaterialTheme.typography.labelMedium,
                    color = palette.muted,
                )
            }
        }
    }
}

@Composable
fun EmptyPlaceholder(
    icon: ImageVector,
    title: String,
    hint: String,
    modifier: Modifier = Modifier,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    val palette = LocalOtteroPalette.current
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = 24.dp, horizontal = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = palette.muted,
        )
        Spacer(Modifier.height(8.dp))
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            color = palette.ink,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            text = hint,
            style = MaterialTheme.typography.bodySmall,
            color = palette.muted,
        )
        if (actionLabel != null && onAction != null) {
            Spacer(Modifier.height(14.dp))
            OutlinedButton(
                onClick = onAction,
                shape = RoundedCornerShape(10.dp),
                border = BorderStroke(1.dp, palette.border),
            ) {
                Text(
                    text = actionLabel,
                    style = MaterialTheme.typography.labelLarge,
                    color = palette.ink,
                )
            }
        }
    }
}
