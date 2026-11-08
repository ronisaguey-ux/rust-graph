typedef struct Point {
    int x;
    int y;
    unsigned flags;
} Point;

struct Node {
    int value;
    struct Node *next;
};

enum Color {
    RED,
    GREEN = 2,
    BLUE = 4,
};

union Number {
    int i;
    float f;
    double d;
};

struct Rect {
    struct Point origin;
    struct Point corner;
};

static struct Node *push(struct Node *head, int value) {
    struct Node node;
    node.value = value;
    node.next = head;
    return head;
}

int area(struct Rect r) {
    int width = r.corner.x - r.origin.x;
    int height = r.corner.y - r.origin.y;
    return width * height;
}

int pick(enum Color color) {
    switch (color) {
    case RED:
        return 1;
    case GREEN:
        return 2;
    default:
        return 0;
    }
}
