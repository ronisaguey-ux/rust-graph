const initialState = { todos: [], filter: 'all', nextId: 1 };

export function reducer(state = initialState, action) {
  switch (action.type) {
    case 'add': {
      const todo = { id: state.nextId, text: action.text, done: false };
      return { ...state, todos: [...state.todos, todo], nextId: state.nextId + 1 };
    }
    case 'toggle': {
      const todos = state.todos.map((todo) =>
        todo.id === action.id ? { ...todo, done: !todo.done } : todo
      );
      return { ...state, todos };
    }
    case 'remove':
      return { ...state, todos: state.todos.filter((todo) => todo.id !== action.id) };
    case 'clear':
      return { ...state, todos: state.todos.filter((todo) => !todo.done) };
    case 'filter':
      return { ...state, filter: action.filter };
    default:
      return state;
  }
}

export const visible = (state) => {
  switch (state.filter) {
    case 'open':
      return state.todos.filter((todo) => !todo.done);
    case 'done':
      return state.todos.filter((todo) => todo.done);
    default:
      return state.todos;
  }
};

export const remaining = (state) => state.todos.reduce((total, todo) => (todo.done ? total : total + 1), 0);
