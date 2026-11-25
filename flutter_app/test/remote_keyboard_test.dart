import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:p2p_remote_ui/widgets/remote_keyboard.dart';

void main() {
  test('IME composition is not sent before selection is committed', () {
    const value = TextEditingValue(
      text: '\u200bnihao',
      composing: TextRange(start: 1, end: 6),
    );
    expect(RemoteImeBuffer.committed(value), isEmpty);
    expect(RemoteImeBuffer.committed(const TextEditingValue(
      text: '\u200b你好',
    )), [(kind: 'text', value: '你好')]);
  });
  test('anchor, whitespace and Backspace are distinct', () {
    expect(RemoteImeBuffer.committed(RemoteImeBuffer.initial), isEmpty);
    expect(RemoteImeBuffer.committed(TextEditingValue.empty),
        [(kind: 'key', value: 'backspace')]);
    expect(RemoteImeBuffer.committed(const TextEditingValue(
      text: '\u200b ',
    )), [(kind: 'text', value: ' ')]);
  });
  test('committed newlines and tabs use real named keys', () {
    expect(RemoteImeBuffer.committed(const TextEditingValue(
      text: '\u200ba\nb\tc',
    )), [(kind: 'text', value: 'a'), (kind: 'key', value: 'enter'),
      (kind: 'text', value: 'b'), (kind: 'key', value: 'tab'),
      (kind: 'text', value: 'c')]);
  });
  test('long commits are split on Unicode scalar boundaries', () {
    final text = List.filled(251, '😀').join();
    final result = RemoteImeBuffer.committed(TextEditingValue(
      text: RemoteImeBuffer.anchor + text,
    ));
    expect(result.map((a) => a.value.runes.length), [250, 1]);
    expect(result.map((a) => a.value).join(), text);
  });
}
