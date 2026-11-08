module scanner;

import std.algorithm : sort;
import io = std.stdio;

alias Callback = void delegate(string);

enum limit = 32;

template Registry(T)
{
    struct Entry
    {
        string key;
        T value;
    }
}

struct Rule
{
    string pattern;
    bool enabled = true;
}

bool match(Rule rule, string text)
{
    return rule.enabled && text.length > 0;
}

int scan(Rule[] rules, string text)
{
    int hits = 0;
    size_t index = 0;
    while (index < rules.length)
    {
        Rule rule = rules[index];
        switch (rule.pattern)
        {
        case "password":
        case "secret":
            hits += 1;
            break;
        default:
            break;
        }
        index += 1;
    }
    assert(hits >= 0);
    return hits;
}
