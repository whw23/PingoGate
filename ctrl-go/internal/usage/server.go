// Package usage - gRPC UsageService server (constitution XIX; spec §12D SC-10).
//
// The Rust kernel pushes UsageEvent messages over a client-streaming
// ReportUsage RPC. The server drains the stream, runs the tiktoken
// estimator for events with needs_estimate=true (no provider usage),
// persists each event to the SQLite DB via the Store, and returns a
// single Ack when the stream closes.
//
// Management audit (spec §9, constitution XIX): each persisted event
// emits a slog audit line recording owner + vkey + token counts. The
// request/response body is never logged (constitution XX: Rust pushes
// body_ref only for estimation; Go discards it after counting).
package usage

import (
	"context"
	"io"
	"log/slog"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	pb "github.com/whw23/pingogate/ctrl-go/internal/proto"
)

// Server implements pingogate.UsageServiceServer. It is constructed with
// a Store (DB persistence) and an Estimator (tiktoken-go). Both are
// injected (constitution VII) so tests can substitute fakes.
type Server struct {
	store     *Store
	estimator *Estimator
	log       *slog.Logger
	pb.UnimplementedUsageServiceServer
}

// NewServer constructs a UsageService server. store and estimator must
// be non-nil; log may be nil (defaults to slog.Default).
func NewServer(store *Store, estimator *Estimator, log *slog.Logger) *Server {
	if log == nil {
		log = slog.Default()
	}
	return &Server{store: store, estimator: estimator, log: log}
}

// ReportUsage is the client-streaming RPC the Rust kernel calls to push
// usage events. Each event is persisted immediately (no batching) so a
// Go crash never loses more than the in-flight event. The stream is
// drained until EOF; a malformed event or DB error aborts the stream
// with a gRPC status so the Rust kernel can log and retry.
//
// Audit: every persisted event emits a slog.Info line with owner, vkey,
// provider, model, and token counts. The body_ref is never logged
// (constitution XX). A needs_estimate=true event additionally logs
// the estimated input/output so operators can see estimation volume.
func (s *Server) ReportUsage(stream pb.UsageService_ReportUsageServer) error {
	ctx := stream.Context()
	for {
		event, err := stream.Recv()
		if err == io.EOF {
			return stream.SendAndClose(&pb.Ack{Ok: true})
		}
		if err != nil {
			return status.Errorf(codes.Aborted, "usage: recv: %v", err)
		}
		if err := s.handleEvent(ctx, event); err != nil {
			return status.Errorf(codes.Internal, "usage: handle event: %v", err)
		}
	}
}

// handleEvent applies estimation (when requested) and persists the
// event. Split from ReportUsage so tests can exercise the logic without
// standing up a gRPC stream.
func (s *Server) handleEvent(ctx context.Context, event *pb.UsageEvent) error {
	if event == nil {
		return status.Error(codes.InvalidArgument, "nil event")
	}
	if event.OwnerUserId == "" {
		// In standalone mode the Rust kernel has no virtual-key owner;
		// record under a sentinel so aggregations still work.
		event.OwnerUserId = "standalone"
	}
	estimated := false
	if event.NeedsEstimate {
		input, output := s.estimator.Estimate(event.BodyRef, event.Provider, event.Model)
		// Only overwrite zero fields; provider-supplied numbers win
		// even when needs_estimate is set (partial-usage case).
		if event.InputTokens == 0 {
			event.InputTokens = input
		}
		if event.OutputTokens == 0 {
			event.OutputTokens = output
		}
		estimated = true
		// Body ref is discarded after estimation (constitution XX: no
		// request/response content persists beyond the estimate).
		event.BodyRef = nil
	}
	rowID, err := s.store.Insert(ctx, event)
	if err != nil {
		return err
	}
	s.audit(ctx, event, rowID, estimated)
	return nil
}

// audit emits the slog audit line for a persisted event (spec §9:
// who/what/when; constitution XIX: usage audit records owner+vkey+token
// counts; never the body).
func (s *Server) audit(ctx context.Context, event *pb.UsageEvent, rowID int64, estimated bool) {
	s.log.InfoContext(ctx, "usage persisted",
		slog.String("what", "usage_persist"),
		slog.String("who", event.OwnerUserId),
		slog.String("when", time.Now().UTC().Format(time.RFC3339)),
		slog.String("resource", event.VirtualKeyId),
		slog.Int64("row_id", rowID),
		slog.String("provider", event.Provider),
		slog.String("model", event.Model),
		slog.Int64("input_tokens", event.InputTokens),
		slog.Int64("output_tokens", event.OutputTokens),
		slog.Bool("success", event.Success),
		slog.Bool("estimated", estimated),
	)
}
