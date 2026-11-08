<?php
namespace App\Service;

use App\Models\User;
use RuntimeException;

class UserService
{
    private array $users = [];

    public function add(User $user): void
    {
        $this->users[] = $user;
    }

    public function find(string $name): ?User
    {
        foreach ($this->users as $user) {
            if ($user->name() === $name) {
                return $user;
            }
        }
        return null;
    }

    public function load(array $rows): void
    {
        foreach ($rows as $row) {
            try {
                $user = new User($row["name"]);
                $this->add($user);
            } catch (RuntimeException $error) {
                echo "failed: " . $error->getMessage();
            } finally {
                echo "done";
            }
        }
    }
}
