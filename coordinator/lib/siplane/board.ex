defmodule Siplane.Board do
  use Supervisor
  use Ecto.Schema
  import Ecto.Changeset
  import Ecto.Query

  # ----- Database Model -------------------------------------------------------

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "boards" do
    field :label, :string
    field :location, :string
    field :manufacturer, :string
    field :hwrev, :string
    field :model, :string
    field :runner_token, :string
    field :image_url, :string

    timestamps(type: :utc_datetime)
  end

  @doc false
  def changeset(board, attrs) do
    board
    |> cast(attrs, [:label, :manufacturer, :model, :hwrev, :location, :runner_token])
    |> validate_required([:label, :manufacturer, :model, :hwrev, :location, :runner_token])
  end

  # ----- Public-Facing API ----------------------------------------------------

  # Supervise a registry managing board servers, and a registry of subscribers
  # to board events. We can't combine them in one registry, as we want multiple
  # subscribers for a board, but never multiple servers for a board.
  def start_link(init_arg) do
    Supervisor.start_link(__MODULE__, init_arg, name: __MODULE__)
  end

  # Check whether a runner is currently connected to a board. This
  # will not start a new board server, in case one doesn't exist yet.
  def runner_connected?(board_id) do
    status(board_id) != :disconnected
  end

  def status(board_id) do
    Registry.lookup(__MODULE__.Server.Registry, board_id)
    |> Enum.map(fn {pid, _} ->
      GenServer.call(pid, :get_status)
    end)
    |> List.first(:disconnected)
  end

  # Connect the current process as a runner-connection to this board.
  def connect_runner(board_id, conn_info) do
    pid = get_or_start(board_id)
    GenServer.call(pid, {:connect_runner, conn_info})
  end

  # Retrieve a given number of log messages related to boards.
  #
  # When nil == 0, this loads all log messages related to this board.
  def board_log(board_id, limit \\ 50) do
    # sql_selection = [
    #   where: msg.board_id == ^board_id,
    #   sort_by: msg.inserted_at,
    # ]

    # sql_selection =
    #   if !is_nil(limit) do
    # 	[{:limit, limit} | sql_selection]
    #   else
    # 	sql_selection
    #   end

    query = from bl in Siplane.Board.LogEvent,
      join: l in assoc(bl, :log_event),
      where: bl.board_id == ^board_id,
      order_by: [desc: l.inserted_at]

    query =
      if !is_nil(limit) do
	limit(query, ^limit)
      else
	query
      end

    Siplane.Repo.all(query)
    |> Siplane.Repo.preload(:log_event)
    |> Enum.map(fn board_log_event -> board_log_event.log_event end)
  end

  # Subscribe to events related to a board
  def subscribe(board_id) do
    {:ok, _} = Registry.register(__MODULE__.SubscriberRegistry, board_id, nil)
    :ok
  end

  # ----- Public callbacks -----------------------------------------------------

  # Callback invoked for board-related log events (which have at least
  # one `Siplane.Board.LogEvent` attached to them)
  def log_event(event) do
    for board <- event.log_event_boards do
      Registry.dispatch(__MODULE__.SubscriberRegistry, board.board_id, fn subscribers ->
	for {pid, _} <- subscribers, do: send(pid, {:board_event, board.board_id, :log_event, event})
      end)
    end
  end

  # ----- Private API ----------------------------------------------------------

  defp get_or_start(board_id) do
    # We need to atomically either get a board GenServer from the registry or
    # spawn one. We try to get one first, and if this doesn't work spawn
    # one. When spawning the GenServer, it itself will then try to register
    # itself with the registry. This may fail if there was a concurrent request
    # to register. This is fine, as we don't link the current process to that
    # GenServer, so we simply let it crash. Instead, we'll ask the Registry a
    # second time, which should now hold an instance of that server. If it still
    # doesn't we should crash.
    case Registry.lookup(__MODULE__.Server.Registry, board_id) do
      [{pid, _}] ->
	# The first lookup worked, return immediately
	pid

      _ ->
	# No server started yet, attempt to start one.
	GenServer.start(__MODULE__.Server, board_id)

	# Now, re-check the Registry, which should a reference to a server for
	# this board id now. This may not be the server we just lauched:
	[{pid, _}] = Registry.lookup(__MODULE__.Server.Registry, board_id)

	# Return this reference:
	pid
    end
  end

  # ----- Supervisor Implementation --------------------------------------------

  @impl true
  def init(_init_arg) do
    children = [
      # Registry of board-servers
      {Registry, keys: :unique, name: __MODULE__.Server.Registry},
      # Registry of subscribers to board-events
      {Registry, keys: :duplicate, name: __MODULE__.SubscriberRegistry},
    ]

    Supervisor.init(children, strategy: :one_for_one)
  end
end
