module broken;

struct Good
{
    int value;
}

// An unterminated block comment hides the rest of the file, and the function
// before it is missing its body brace.
int compute(int a, int b)
{
    return a + b;

void tail()
{
    auto x = compute(1, 2);
/* no end here
