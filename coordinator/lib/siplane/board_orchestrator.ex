defmodule Siplane.BoardOrchestrator do
  require Logger
  use GenServer

  def start(board_id) do
    GenServer.start(
      __MODULE__,
      board_id,
      name: {:via, Registry, {__MODULE__.Registry, board_id}}
    )
  end

  def get_or_start(board_id) do
    pid = Registry.lookup(__MODULE__.Registry, board_id)
    |> Enum.map(fn {pid, _} -> pid end)
    |> List.first(nil)

    if pid == nil do
      __MODULE__.start(board_id)
    else
      {:ok, pid}
    end
  end

  def runner_connected?(pid) do
    GenServer.call(pid, :runner_connected)
  end

  def connect_runner(pid) do
    Process.monitor(pid)
    GenServer.call(pid, :connect_runner)
  end

  def log_event(pid, event) do
    GenServer.cast(pid, {:log_event, event})
  end

  defp runner_connected_priv?(state) do
    state.runner_proc == nil || !(Process.alive? state.runner_proc)
  end

  defp shutdown_check(state) do
    # If we have a runner attached, don't shut down the orchestrator.
    runner_connected_priv? state
  end

  @impl true
  def init(board_id) do
    # Periodically schedule checks whether to shut down this orchestrator
    Process.send_after(self(), :shutdown_check, 5_000)

    {:ok, %{
	board_id: board_id,
	runner_proc: nil,
	log_event_proc: nil,
    }}
  end

  @impl true
  def handle_info(:shutdown_check, state) do
    if shutdown_check state do
      {:stop, :shutdown, state}
    else
      {:noreply, state}
    end
  end


  @impl true
  def handle_call(:runner_connected, _from, state) do
    {:reply, runner_connected_priv?(state), state}
  end

  @impl true
  def handle_call(:connect_runner, {from_pid, _}, state) do
    Siplane.Log.info(%Siplane.Log{
	  severity: 2,
	  event_type: "board_runner_connected",
	  event_version: "0.1",
	  message: "Runner has established connection to board orchestrator for board #{UUID.binary_to_string! state.board_id}.",
	  data: %{board_id: UUID.binary_to_string! state.board_id},
	  log_event_boards: [%Siplane.Board.LogEvent{
	    board_id: Ecto.UUID.load!(state.board_id),
	    public: true,
	    owner_visible: true,
	  }],
    })

    # Indicate to the runner connection process that it has been
    # successfully connected to an orchestrator instance.
    #
    # TODO: if we had an old connection for this runner, we should
    # inform it that it is now detached.
    :ok = Process.send(from_pid, {:runner_conn, {self(), state.board_id}, :msg, :registered}, [])

    # Update the state to store the new runner connection.
    {
      :reply,
      # Return a last will and testament, to be forwarded to client if
      # the orchestrator were to die:
      {:ok, %{ "reason" => "Board orchestrator shut down." }},
      %{state | runner_proc: from_pid }
    }
  end

  @impl true
  def handle_cast({:log_event, event}, state) do
    if !is_nil(state.log_event_proc) do
      Process.send(state.log_event_proc, {:board_log_event, {self(), state.board_id}, event}, [])
    end

    {:noreply, state}
  end

end
