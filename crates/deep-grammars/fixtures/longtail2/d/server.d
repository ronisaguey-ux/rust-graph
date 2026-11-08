module server;

import std.stdio;

class Handler
{
    private int count;
    private string prefix;

    this(string prefix)
    {
        this.prefix = prefix;
        this.count = 0;
    }

    string render(string name)
    {
        count += 1;
        return prefix ~ "/" ~ name;
    }

    int hits() const
    {
        return count;
    }
}

int serve(Handler[] handlers, int limit)
{
    int total = 0;
    for (int i = 0; i < limit; i++)
    {
        if (i >= handlers.length)
        {
            break;
        }
        total += i;
    }
    return total;
}
