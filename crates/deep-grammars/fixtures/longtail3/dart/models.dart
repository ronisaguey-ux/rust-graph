library models;

enum Status {
  idle,
  running,
  done;

  String label() {
    return this.toString();
  }
}

class Point {
  final double x;
  final double y;

  Point(this.x, this.y);

  double distance(Point other) {
    final dx = this.x - other.x;
    final dy = this.y - other.y;
    return dx * dx + dy * dy;
  }
}

abstract class Shape {
  double area() => 0.0;

  String describe() {
    return 'area ' + area().toString();
  }
}
