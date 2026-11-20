import 'package:material_ui/material_ui.dart';
import 'package:window_manager/window_manager.dart';

import '../app_theme.dart';

class DesktopWindowFrame extends StatelessWidget {
  const DesktopWindowFrame({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return DragToResizeArea(
      child: Column(
        children: [
          const _DesktopTitleBar(),
          Expanded(child: child),
        ],
      ),
    );
  }
}

class _DesktopTitleBar extends StatefulWidget {
  const _DesktopTitleBar();

  @override
  State<_DesktopTitleBar> createState() => _DesktopTitleBarState();
}

class _DesktopTitleBarState extends State<_DesktopTitleBar> {
  Window? get _window => WindowManager.instance.getCurrent();

  late bool _isMaximized = _window?.isMaximized ?? false;

  void _toggleMaximize() {
    final window = _window;
    if (window == null) return;

    final shouldMaximize = !window.isMaximized;
    if (shouldMaximize) {
      window.maximize();
    } else {
      window.unmaximize();
    }
    setState(() => _isMaximized = shouldMaximize);
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 42,
      decoration: const BoxDecoration(
        color: P2PColors.background,
        border: Border(
          bottom: BorderSide(color: P2PColors.outline, width: 0.7),
        ),
      ),
      child: Row(
        children: [
          Expanded(
            child: DragToMoveArea(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onDoubleTap: _toggleMaximize,
                child: Padding(
                  padding: const EdgeInsets.only(left: 16, right: 12),
                  child: Row(
                    children: [
                      Container(
                        width: 24,
                        height: 24,
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(7),
                          gradient: const LinearGradient(
                            colors: [P2PColors.mint, Color(0xFF3E91C7)],
                            begin: Alignment.topLeft,
                            end: Alignment.bottomRight,
                          ),
                        ),
                        child: const Icon(
                          Icons.bolt_rounded,
                          color: Color(0xFF08211F),
                          size: 16,
                        ),
                      ),
                      const SizedBox(width: 9),
                      const Text(
                        'P2P Remote',
                        style: TextStyle(
                          color: P2PColors.text,
                          fontSize: 12,
                          fontWeight: FontWeight.w700,
                          letterSpacing: 0.2,
                        ),
                      ),
                      const SizedBox(width: 10),
                      const Text(
                        'V0.17',
                        style: TextStyle(
                          color: P2PColors.muted,
                          fontSize: 10,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
          _WindowActionButton(
            tooltip: '最小化',
            icon: Icons.remove_rounded,
            onPressed: () => _window?.minimize(),
          ),
          _WindowActionButton(
            tooltip: _isMaximized ? '还原' : '最大化',
            icon: _isMaximized
                ? Icons.filter_none_rounded
                : Icons.crop_square_rounded,
            onPressed: _toggleMaximize,
          ),
          _WindowActionButton(
            tooltip: '关闭',
            icon: Icons.close_rounded,
            isClose: true,
            onPressed: () => _window?.close(),
          ),
        ],
      ),
    );
  }
}

class _WindowActionButton extends StatefulWidget {
  const _WindowActionButton({
    required this.tooltip,
    required this.icon,
    required this.onPressed,
    this.isClose = false,
  });

  final String tooltip;
  final IconData icon;
  final VoidCallback onPressed;
  final bool isClose;

  @override
  State<_WindowActionButton> createState() => _WindowActionButtonState();
}

class _WindowActionButtonState extends State<_WindowActionButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final background = _hovered
        ? (widget.isClose
            ? const Color(0xFFE04444)
            : const Color(0xFF22334A))
        : Colors.transparent;

    return Tooltip(
      message: widget.tooltip,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hovered = true),
        onExit: (_) => setState(() => _hovered = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onPressed,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 120),
            width: 46,
            height: 42,
            color: background,
            alignment: Alignment.center,
            child: Icon(
              widget.icon,
              size: 16,
              color: _hovered && widget.isClose
                  ? Colors.white
                  : P2PColors.text,
            ),
          ),
        ),
      ),
    );
  }
}
