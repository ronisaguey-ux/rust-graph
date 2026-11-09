// TSX: the same element grammar, with a typed attribute expression and a
// type argument on a tag's call.
type Props = { user: User };
function Panel({ user }: Props) {
  return <Button onClick={() => select(user)} label={`hi ${user.name}`}>{format<User>(user)}</Button>;
}
const select = <Select<Option> value={pick(selected)} />;
