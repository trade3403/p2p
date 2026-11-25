import 'dart:async';
import 'dart:collection';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../services/remote_controller.dart';

/// Transient local anchor for detecting Backspace without a local document.
/// Never send the anchor or uncommitted Pinyin composition.
class RemoteImeBuffer {
  static const anchor = '\u200b';
  static const initial = TextEditingValue(
    text: anchor, selection: TextSelection.collapsed(offset: 1),
  );

  static bool composing(TextEditingValue value) =>
      value.composing.isValid && !value.composing.isCollapsed;

  static List<({String kind, String value})> committed(TextEditingValue value) {
    if (composing(value) || value.text == anchor) return [];
    if (value.text.isEmpty) return [(kind: 'key', value: 'backspace')];
    final result = <({String kind, String value})>[];
    final text = value.text.startsWith(anchor)
        ? value.text.substring(anchor.length) : value.text;
    final buffer = <int>[];
    void flush() {
      if (buffer.isNotEmpty) {
        result.add((kind: 'text', value: String.fromCharCodes(buffer)));
        buffer.clear();
      }
    }
    for (final rune in text.runes) {
      if (rune == 10 || rune == 13 || rune == 9) {
        flush();
        result.add((kind: 'key', value: rune == 9 ? 'tab' : 'enter'));
      } else {
        buffer.add(rune);
        if (buffer.length == 250) flush();
      }
    }
    flush();
    return result;
  }
}

/// Explicit opt-in system IME inside the fullscreen video route.
class RemoteKeyboard extends StatefulWidget {
  const RemoteKeyboard({super.key, required this.controller});
  final P2PRemoteController controller;

  @override
  State<RemoteKeyboard> createState() => _RemoteKeyboardState();
}

class _RemoteKeyboardState extends State<RemoteKeyboard>
    with WidgetsBindingObserver {
  final _input = TextEditingController.fromValue(RemoteImeBuffer.initial);
  final _focus = FocusNode(debugLabel: 'remote-ime');
  final _pending = Queue<({String kind, String value})>();
  bool _open = false;
  bool _resetting = false;
  bool _draining = false;
  bool _failed = false;
  int _epoch = 0;

  bool get _allowed => widget.controller.viewerConnected &&
      widget.controller.viewerAllowsInput;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    widget.controller.addListener(_permissionChanged);
    _input.addListener(_changed);
    _focus.addListener(_focusChanged);
  }

  void _permissionChanged() {
    if (_open && !_allowed) _close();
  }

  void _focusChanged() {
    if (_open && !_focus.hasFocus) _close();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.resumed && _open) _close();
  }

  void _reset() {
    _resetting = true;
    _input.value = RemoteImeBuffer.initial;
    _resetting = false;
  }

  void _toggle() {
    if (_open) {
      _close();
    } else if (_allowed) {
      _epoch++;
      _reset();
      setState(() { _open = true; _failed = false; });
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !_open || !_allowed) return;
        _focus.requestFocus();
      });
    }
  }

  void _close({bool failed = false}) {
    if (!_open) return;
    _epoch++;
    _pending.clear();
    setState(() { _open = false; _failed = failed; });
    _focus.unfocus();
    _reset();
  }

  void _changed() {
    if (_resetting || !_open || !_focus.hasFocus || !_allowed) return;
    final value = _input.value;
    if (RemoteImeBuffer.composing(value)) return;
    final actions = RemoteImeBuffer.committed(value);
    if (value.text != RemoteImeBuffer.anchor) _reset();
    _enqueue(actions);
  }

  void _enqueue(List<({String kind, String value})> actions) {
    if (!_open || !_allowed || actions.isEmpty) return;
    if (ModalRoute.of(context)?.isCurrent != true ||
        _pending.length + actions.length > 64) {
      _close(failed: true);
      return;
    }
    _pending.addAll(actions);
    unawaited(_drain());
  }

  Future<void> _drain() async {
    if (_draining) return;
    _draining = true;
    final epoch = _epoch;
    try {
      while (mounted && _open && _allowed && epoch == _epoch &&
          _pending.isNotEmpty) {
        final action = _pending.removeFirst();
        final ok = action.kind == 'key'
            ? await widget.controller.sendKey(action.value)
            : await widget.controller.sendText(action.value);
        if (!mounted || epoch != _epoch) return;
        if (!ok) {
          _close(failed: true);
          return;
        }
      }
    } catch (_) {
      if (mounted && epoch == _epoch) _close(failed: true);
    } finally {
      _draining = false;
      if (mounted && _open && _pending.isNotEmpty) unawaited(_drain());
    }
  }

  void _key(String key) {
    if (RemoteImeBuffer.composing(_input.value)) _reset();
    _enqueue([(kind: 'key', value: key)]);
  }

  @override
  void dispose() {
    _epoch++;
    _pending.clear();
    WidgetsBinding.instance.removeObserver(this);
    widget.controller.removeListener(_permissionChanged);
    _input.removeListener(_changed);
    _focus.removeListener(_focusChanged);
    _input.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Stack(
    children: [
      Row(
        children: [
          TextButton(
            onPressed: _allowed ? _toggle : null,
            child: Text(_open ? '收起键盘' : '键盘'),
          ),
          if (_open)
            Expanded(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Row(
                  children: [
                    for (final (label, key) in const [
                      ('退格', 'backspace'), ('回车', 'enter'),
                      ('空格', 'space'), ('Tab', 'tab'), ('Esc', 'escape'),
                      ('←', 'left'), ('↑', 'up'), ('↓', 'down'), ('→', 'right'),
                      ('Del', 'delete'), ('Ctrl+A', 'ctrl_a'), ('Ctrl+C', 'ctrl_c'),
                      ('Ctrl+V', 'ctrl_v'), ('Ctrl+X', 'ctrl_x'),
                      ('Ctrl+Z', 'ctrl_z'), ('Ctrl+Y', 'ctrl_y'),
                    ])
                      TextButton(onPressed: () => _key(key), child: Text(label)),
                  ],
                ),
              ),
            )
          else if (_failed)
            const Expanded(child: Text('键盘输入已停止，请重新打开')),
        ],
      ),
      if (_open)
        Positioned(
          left: 0, top: 0, width: 2, height: 2,
          child: IgnorePointer(
            child: ExcludeSemantics(
              child: Opacity(
                opacity: 0.01,
                child: EditableText(
                  controller: _input,
                  focusNode: _focus,
                  style: const TextStyle(fontSize: 12),
                  cursorColor: const Color(0x00000000),
                  backgroundCursorColor: const Color(0x00000000),
                  showCursor: false,
                  keyboardType: TextInputType.multiline,
                  textInputAction: TextInputAction.newline,
                  maxLines: null,
                  autocorrect: false,
                  enableSuggestions: true,
                  enableIMEPersonalizedLearning: false,
                  enableInteractiveSelection: false,
                ),
              ),
            ),
          ),
        ),
    ],
  );
}
