<?php
namespace App\Models;

trait HasName
{
    public function name(): string
    {
        return $this->name;
    }
}

enum Status: string
{
    case Active = "active";
    case Inactive = "inactive";
}

final class User
{
    use HasName;

    const DEFAULT_ROLE = "member";

    private ?string $email = null;
    public static int $count = 0;

    public function __construct(private string $name)
    {
        self::$count++;
    }

    public static function make(string $name): self
    {
        return new self($name);
    }

    public function email(): ?string
    {
        return $this->email?->value;
    }
}
