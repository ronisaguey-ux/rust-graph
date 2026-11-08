import 'dart:async';
import 'package:http/http.dart' as http;

part 'server.g.dart';

/// A tiny HTTP server, enough to exercise a class with fields, a constructor
/// and an async method.
class Server {
  final int port = 8080;
  String host = 'localhost';

  Server(int port) : port = port;

  start() async {
    final socket = await open(this.port);
    if (socket == null) {
      throw Exception('no socket on port ' + this.port.toString());
    }
    for (var attempt = 0; attempt < 3; attempt++) {
      print('tick ' + attempt.toString());
    }
    while (socket.isOpen) {
      socket.write('ok');
    }
  }

  close() {
    this.host = '';
  }
}

void main() async {
  final server = Server(8080);
  await server.start();
}
