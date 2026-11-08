module build.server;

import std.stdio;
import std.string : split, strip;
import std.conv : to;

enum Mode
{
    Debug,
    Release,
}

struct Target
{
    string name;
    int port = 8080;
    bool verbose;

    int address()
    {
        return port + 1;
    }
}

int parse_port(string text)
{
    int value = 0;
    foreach (char c; text)
    {
        if (c < '0' || c > '9')
        {
            throw new Exception("bad digit");
        }
        value = value * 10 + (c - '0');
    }
    return value;
}

void main(string[] args)
{
    auto mode = Mode.Release;
    Target target = Target("api", 9000, true);
    writeln("starting ", target.name, " on ", target.port);
    int port = parse_port("9000");
    if (port != target.port)
    {
        writeln("mismatch");
    }
    else
    {
        writeln("ok");
    }
}
