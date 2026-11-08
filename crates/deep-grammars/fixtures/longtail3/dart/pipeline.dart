import 'dart:convert';

const limit = 100;
var counter = 0;
final tags = ['a', 'b'];

String parseLines(String input) {
  final lines = input.split('\n');
  final out = [];
  for (var index = 0; index < lines.length; index++) {
    if (lines[index].length > limit) {
      continue;
    }
    out.add(lines[index].trim());
  }
  return out.join(',');
}

int total(int count) {
  var sum = 0;
  for (var i = 0; i < count; i++) {
    sum += i;
  }
  return sum;
}

void run() {
  try {
    final text = convert(limit);
    print(parseLines(text));
  } on StateError {
    print('failed');
  } finally {
    counter += 1;
  }
}
