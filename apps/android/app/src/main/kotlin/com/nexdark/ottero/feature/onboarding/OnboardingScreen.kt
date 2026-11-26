package com.nexdark.ottero.feature.onboarding

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Spa
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nexdark.ottero.R
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference

@Composable
fun OnboardingScreen(
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    onComplete: () -> Unit,
) {
    val palette = LocalOtteroPalette.current

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = palette.canvas,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .systemBarsPadding(),
            contentAlignment = Alignment.TopCenter,
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 480.dp)
                    .fillMaxSize()
                    .padding(horizontal = 24.dp),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        modifier = Modifier
                            .size(32.dp)
                            .clip(RoundedCornerShape(9.dp)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Image(
                            painter = painterResource(id = R.drawable.ic_launcher_foreground),
                            contentDescription = "Ottero",
                            contentScale = ContentScale.Crop,
                            modifier = Modifier
                                .fillMaxSize()
                                .graphicsLayer(scaleX = 1.48f, scaleY = 1.48f),
                        )
                    }
                    Spacer(Modifier.width(10.dp))
                    Text(
                        text = "Ottero",
                        style = MaterialTheme.typography.titleMedium.copy(
                            fontWeight = FontWeight.SemiBold,
                        ),
                        color = palette.ink,
                    )
                }

                Column(
                    modifier = Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.Center,
                ) {
                    Spacer(Modifier.height(24.dp))

                    Text(
                        text = "Your notes, tasks,\nand habits.",
                        style = MaterialTheme.typography.headlineLarge.copy(
                            fontSize = 34.sp,
                            lineHeight = 40.sp,
                            letterSpacing = (-0.8).sp,
                            fontWeight = FontWeight.SemiBold,
                        ),
                        color = palette.ink,
                    )

                    Spacer(Modifier.height(12.dp))

                    Text(
                        text = "Stored locally on this device.",
                        style = MaterialTheme.typography.bodyLarge,
                        color = palette.muted,
                    )

                    Spacer(Modifier.height(36.dp))

                    Column(verticalArrangement = Arrangement.spacedBy(22.dp)) {
                        OnboardingFeatureRow(
                            icon = Icons.Outlined.Description,
                            title = "Notes",
                            subtitle = "Write and search across all your notes.",
                        )
                        OnboardingFeatureRow(
                            icon = Icons.Outlined.CheckCircle,
                            title = "Tasks",
                            subtitle = "Quick-add with dates and priorities.",
                        )
                        OnboardingFeatureRow(
                            icon = Icons.Outlined.Spa,
                            title = "Habits",
                            subtitle = "Daily check-ins and targets.",
                        )
                    }

                    Spacer(Modifier.height(24.dp))
                }

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(bottom = 24.dp),
                ) {
                    ThemeSegmentedControl(
                        selected = themePreference,
                        onSelect = onThemeChange,
                    )

                    Spacer(Modifier.height(16.dp))

                    Button(
                        onClick = onComplete,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(54.dp),
                        shape = RoundedCornerShape(14.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = palette.ink,
                            contentColor = palette.card,
                        ),
                    ) {
                        Text(
                            text = "Continue",
                            style = MaterialTheme.typography.labelLarge.copy(
                                fontSize = 15.sp,
                                fontWeight = FontWeight.Medium,
                            ),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun OnboardingFeatureRow(
    icon: ImageVector,
    title: String,
    subtitle: String,
) {
    val palette = LocalOtteroPalette.current
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = palette.ink,
            modifier = Modifier
                .padding(top = 2.dp)
                .size(20.dp),
        )
        Spacer(Modifier.width(14.dp))
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold),
                color = palette.ink,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = palette.muted,
            )
        }
    }
}

@Composable
private fun ThemeSegmentedControl(
    selected: ThemePreference,
    onSelect: (ThemePreference) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        color = palette.recessed,
        border = BorderStroke(1.dp, palette.border),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            ThemePreference.entries.forEach { pref ->
                val isSelected = pref == selected
                Surface(
                    modifier = Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(9.dp))
                        .clickable { onSelect(pref) },
                    shape = RoundedCornerShape(9.dp),
                    color = if (isSelected) palette.card else palette.recessed,
                    shadowElevation = if (isSelected && !palette.isDark) 1.dp else 0.dp,
                ) {
                    Text(
                        text = pref.name,
                        style = MaterialTheme.typography.labelLarge.copy(
                            fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Normal,
                        ),
                        color = if (isSelected) palette.ink else palette.muted,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(vertical = 9.dp),
                    )
                }
            }
        }
    }
}
