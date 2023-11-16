defmodule Siplane.Board.Server do
  use GenServer

  defstruct [
    :board_id,
    :runner_pid,
    :board_state,
  ]

  # This module solely contains the GenServer implementation. The public-facing
  # API is provided by the Siplane.Board module (which manages these servers,
  # plus subscribers to board-related events).

  defp request_runner_status(state) do
    if !is_nil(state.runner_pid) do
      send(state.runner_pid, {:board_event, state.board_id, :runner_msg, %{type: :update_state}})
    end
  end

  # ----- GenServer Implementation ---------------------------------------------

  @impl true
  def init(board_id) do
    # Register this server before returning. If we race with another server
    # starting for the same board, exit immediately without returning an error
    # (by returning :ignore).
    case Registry.register(__MODULE__.Registry, board_id, nil) do
      {:error, {:already_registered, _}} ->
	# We raced, ignore this start request:
	:ignore

      {:ok, _} ->
	# We started & are successfully registered.

	# TODO: at some point we should cleanup old servers, when they have no event
	# subscribers or other dependencies any more. However, we need to make sure
	# that this doesn't race with any requests (i.e. don't shut down in between
	# starting the server and servicing a request). We may be able to enforce
	# this consistency as long as all other entities interact with this server
	# through the public-facing interface of this module.

	{
	  :ok,
	  %__MODULE__{
	    board_id: board_id,
	    runner_pid: nil,
	    board_state: :disconnected,
	  }
	}
    end
  end

  @impl true
  def handle_call(:get_state, _from, state) do
    {
      :reply,
      state.board_state,
      state,
    }
  end

  @impl true
  def handle_call({:connect_runner, conn_info}, {runner_pid, _}, state) do
    # We want to log that the runner disconnected (and update our
    # state), thus monitor the runner connection process. This will
    # still deliver a message even if the process is already dead:
    Process.monitor(runner_pid)

    Siplane.Log.info(
      %Siplane.Log{
	severity: Siplane.Log.severity_info,
	event_type: "board_runner_connected",
	event_version: "0.1",
	message: "Runner has established connection for board ${board_id}.",
	data: %{
	  board_id: UUID.binary_to_string!(state.board_id),
	  conn_info: conn_info,
	},
	log_event_boards: [
	  %Siplane.Board.LogEvent{
	    board_id: Ecto.UUID.load!(state.board_id),
	    public: true,
	    owner_visible: true,
          }
	],
      }
    )

    state = %{ state | runner_pid: runner_pid, board_state: :unknown }

    request_runner_status(state)

    {
      :reply,
      {
	:ok,
	self(),
	# TODO: craft an appropriate last will and testament message
	"oops I'm dead!",
      },
      state,
    }
  end

  @impl true
  def handle_call({:update_state, board_state}, _from, state) do
    if state.board_state != board_state do
      Siplane.Log.info(
	%Siplane.Log{
	  severity: Siplane.Log.severity_info,
	  event_type: "board_runner_state_changed",
	  event_version: "0.1",
	  message: "Board ${board_id} has changed state from ${old_state} to ${new_state}.",
	  data: %{
	    board_id: UUID.binary_to_string!(state.board_id),
	    old_state: state.board_state,
	    new_state: board_state,
	  },
	  log_event_boards: [
	    %Siplane.Board.LogEvent{
	      board_id: Ecto.UUID.load!(state.board_id),
	      public: true,
	      owner_visible: true,
}
	  ],
	}
      )
    end

    {:reply, :ok, %{ state | board_state: board_state } }
  end

  @impl true
  def handle_info({:DOWN, _ref, :process, runner_pid, _type}, %{ runner_pid: runner_pid } = state) do
    Siplane.Log.info(
      %Siplane.Log{
	severity: Siplane.Log.severity_info,
	event_type: "board_runner_disconnected",
	event_version: "0.1",
	message: "The connection to the runner for board ${board_id} has been lost.",
	data: %{
	  board_id: UUID.binary_to_string!(state.board_id),
	},
	log_event_boards: [
	  %Siplane.Board.LogEvent{
	    board_id: Ecto.UUID.load!(state.board_id),
	    public: true,
	    owner_visible: true,
          }
	],
      }
    )

    {
      :noreply,
      %{
	state | runner_pid: nil, board_state: :disconnected
      },
    }
  end
end
