import 'package:material_ui/material_ui.dart';

abstract final class P2PColors {
  static const background = Color(0xFF0A111D);
  static const sidebar = Color(0xFF0D1828);
  static const panel = Color(0xFF142238);
  static const panelRaised = Color(0xFF1B2D45);
  static const panelSoft = Color(0xFF101D2F);
  static const outline = Color(0xFF2A405B);
  static const mint = Color(0xFF58D6C5);
  static const mintDeep = Color(0xFF174B48);
  static const blue = Color(0xFF527BFA);
  static const amber = Color(0xFFF1B96F);
  static const text = Color(0xFFF2F6FC);
  static const muted = Color(0xFF9EADC2);
  static const danger = Color(0xFFE27C83);
}

ThemeData buildP2PTheme() {
  final scheme = ColorScheme.fromSeed(
    seedColor: P2PColors.mint,
    brightness: Brightness.dark,
  ).copyWith(
    primary: P2PColors.mint,
    onPrimary: const Color(0xFF06201D),
    secondary: P2PColors.blue,
    surface: P2PColors.panel,
    onSurface: P2PColors.text,
    outline: P2PColors.outline,
    error: P2PColors.danger,
  );

  final border = OutlineInputBorder(
    borderRadius: BorderRadius.circular(13),
    borderSide: const BorderSide(color: P2PColors.outline),
  );

  return ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    colorScheme: scheme,
    scaffoldBackgroundColor: P2PColors.background,
    dividerColor: P2PColors.outline,
    visualDensity: VisualDensity.standard,
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: P2PColors.panelSoft,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 15),
      border: border,
      enabledBorder: border,
      focusedBorder: border.copyWith(
        borderSide: const BorderSide(color: P2PColors.mint, width: 1.4),
      ),
      hintStyle: const TextStyle(color: P2PColors.muted),
      labelStyle: const TextStyle(color: P2PColors.muted),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: P2PColors.mint,
        foregroundColor: const Color(0xFF06201D),
        minimumSize: const Size(44, 46),
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 13),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        textStyle: const TextStyle(fontWeight: FontWeight.w700),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: P2PColors.text,
        side: const BorderSide(color: P2PColors.outline),
        minimumSize: const Size(42, 44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: P2PColors.mint,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
      ),
    ),
  );
}
