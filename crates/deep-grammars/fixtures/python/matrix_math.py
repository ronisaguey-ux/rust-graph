from math import isclose


class Matrix:
    def __init__(self, rows, columns, fill=0.0):
        self.rows = rows
        self.columns = columns
        self.cells = [[fill] * columns for _ in range(rows)]

    def __getitem__(self, index):
        row, column = index
        return self.cells[row][column]

    def __setitem__(self, index, value):
        row, column = index
        self.cells[row][column] = value

    def __matmul__(self, other):
        if self.columns != other.rows:
            raise ValueError("shape mismatch")
        result = Matrix(self.rows, other.columns)
        for row in range(self.rows):
            for column in range(other.columns):
                total = 0.0
                for inner in range(self.columns):
                    total += self[row, inner] * other[inner, column]
                result[row, column] = total
        return result

    def transpose(self):
        flipped = Matrix(self.columns, self.rows)
        for row in range(self.rows):
            for column in range(self.columns):
                flipped[column, row] = self[row, column]
        return flipped

    def __eq__(self, other):
        if not isinstance(other, Matrix):
            return NotImplemented
        return all(
            isclose(a, b, rel_tol=1e-9)
            for mine, theirs in zip(self.cells, other.cells)
            for a, b in zip(mine, theirs)
        )


def identity(size):
    result = Matrix(size, size)
    for index in range(size):
        result[index, index] = 1.0
    return result
