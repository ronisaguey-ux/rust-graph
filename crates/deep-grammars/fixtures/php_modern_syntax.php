<?php
declare(strict_types=1);

namespace App\Modern;

use App\Models\{User, Post as BlogPost};
use function App\Helpers\slugify;

#[Attribute(Attribute::TARGET_CLASS)]
final class Controller
{
    use LogsActivity, FormatsOutput {
        LogsActivity::log insteadof FormatsOutput;
        FormatsOutput::log as public formatted;
    }

    public function __construct(private readonly User $user, protected ?int $limit = null)
    {
    }

    public function label(int|string $key): string
    {
        return match (true) {
            1, 2 => "low",
            default => "high",
        };
    }
}

$slug = fn(string $value): string => trim($value);
$callback = static function (User $user) use (&$seen): bool {
    return isset($seen[$user->id]);
};
?>
<p class="note"><?= $slug("Hello") ?></p>
