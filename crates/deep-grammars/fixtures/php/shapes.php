<?php
namespace App\Shapes;

use App\Contracts\Shape;

interface ShapeInterface
{
    public function area(): float;
}

abstract class BaseShape implements ShapeInterface
{
    protected string $name;

    public function __construct(string $name)
    {
        $this->name = $name;
    }

    public function name(): string
    {
        return $this->name;
    }
}

class Circle extends BaseShape
{
    private float $radius;

    public function __construct(string $name, float $radius)
    {
        parent::__construct($name);
        $this->radius = $radius;
    }

    public function area(): float
    {
        return 3.14159 * $this->radius ** 2;
    }
}
