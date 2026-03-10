// Copyright The Mumble Developers. All rights reserved.
// Use of this source code is governed by a BSD-style license
// that can be found in the LICENSE file at the root of the
// Mumble source tree or at <https://www.mumble.info/LICENSE>.

#include "WebSocketConnection.h"
#include "Mumble.pb.h"
#include "SSL.h"

#include <QtCore/QtEndian>
#include <QtNetwork/QHostAddress>
#include <QtWebSockets/QWebSocket>

/// Maximum Mumble message payload size in bytes (matches the TCP Connection limit).
static constexpr int MAX_MUMBLE_MESSAGE_SIZE = 0x7fffff;

WebSocketConnection::WebSocketConnection(QObject *parent, QWebSocket *socket) : QObject(parent), m_socket(socket) {
	m_socket->setParent(this);
	bDisconnectedEmitted = false;
	csCrypt              = std::make_unique< CryptStateOCB2 >();
	m_lastPacket.restart();

	connect(m_socket, &QWebSocket::binaryMessageReceived, this,
			&WebSocketConnection::socketBinaryMessageReceived);
	connect(m_socket, qOverload< QAbstractSocket::SocketError >(&QWebSocket::error), this,
			&WebSocketConnection::socketError);
	connect(m_socket, &QWebSocket::disconnected, this, &WebSocketConnection::socketDisconnected);
	connect(m_socket, &QWebSocket::sslErrors, this, &WebSocketConnection::socketSslErrors);
}

/// Each incoming binary WebSocket message is a complete Mumble TCP frame:
///   [2-byte type][4-byte length][payload]
void WebSocketConnection::socketBinaryMessageReceived(const QByteArray &msg) {
	m_lastPacket.restart();

	// We need at least the 6-byte header.
	if (msg.size() < 6) {
		return;
	}

	const unsigned char *data = reinterpret_cast< const unsigned char * >(msg.constData());
	Mumble::Protocol::TCPMessageType type =
		static_cast< Mumble::Protocol::TCPMessageType >(qFromBigEndian< quint16 >(&data[0]));
	const int len = qFromBigEndian< qint32 >(&data[2]);

	if (len < 0 || len > MAX_MUMBLE_MESSAGE_SIZE) {
		qWarning() << "WebSocketConnection: received oversized or invalid message (len =" << len << ")";
		disconnectSocket(true);
		return;
	}

	if (msg.size() < 6 + len) {
		qWarning() << "WebSocketConnection: message too short for declared length";
		return;
	}

	emit message(type, msg.mid(6, len));
}

void WebSocketConnection::socketError(QAbstractSocket::SocketError err) {
	emit connectionClosed(err, m_socket->errorString());
}

void WebSocketConnection::socketSslErrors(const QList< QSslError > &errors) {
	emit handleSslErrors(errors);
}

void WebSocketConnection::socketDisconnected() {
	emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
}

// static
void WebSocketConnection::messageToNetwork(const ::google::protobuf::Message &msg,
										   Mumble::Protocol::TCPMessageType msgType, QByteArray &cache) {
#if GOOGLE_PROTOBUF_VERSION >= 3004000
	const std::size_t len = msg.ByteSizeLong();
#else
	const std::size_t len = static_cast< std::size_t >(msg.ByteSize());
#endif
	if (len > MAX_MUMBLE_MESSAGE_SIZE)
		return;
	cache.resize(static_cast< int >(len + 6));
	unsigned char *uc = reinterpret_cast< unsigned char * >(cache.data());
	qToBigEndian< quint16 >(static_cast< quint16 >(msgType), &uc[0]);
	qToBigEndian< quint32 >(static_cast< unsigned int >(len), &uc[2]);
	msg.SerializeToArray(uc + 6, static_cast< int >(len));
}

void WebSocketConnection::sendMessage(const ::google::protobuf::Message &msg,
									  Mumble::Protocol::TCPMessageType msgType, QByteArray &cache) {
	if (cache.isEmpty()) {
		messageToNetwork(msg, msgType, cache);
	}
	sendMessage(cache);
}

/// Send a pre-framed Mumble TCP message as a binary WebSocket frame.
void WebSocketConnection::sendMessage(const QByteArray &data) {
	if (!data.isEmpty()) {
		m_socket->sendBinaryMessage(data);
	}
}

void WebSocketConnection::disconnectSocket(bool force) {
	if (m_socket->state() == QAbstractSocket::UnconnectedState) {
		emit connectionClosed(QAbstractSocket::UnknownSocketError, QString());
		return;
	}

	if (force) {
		m_socket->abort();
	} else {
		m_socket->close();
	}
}

void WebSocketConnection::forceFlush() {
	// QWebSocket sends messages immediately; no explicit flush needed.
}

void WebSocketConnection::proceedAnyway() {
	m_socket->ignoreSslErrors();
}

qint64 WebSocketConnection::activityTime() const {
	return m_lastPacket.elapsed();
}

void WebSocketConnection::resetActivityTime() {
	m_lastPacket.restart();
}

QHostAddress WebSocketConnection::peerAddress() const {
	return m_socket->peerAddress();
}

quint16 WebSocketConnection::peerPort() const {
	return m_socket->peerPort();
}

QHostAddress WebSocketConnection::localAddress() const {
	return m_socket->localAddress();
}

quint16 WebSocketConnection::localPort() const {
	return m_socket->localPort();
}

QList< QSslCertificate > WebSocketConnection::peerCertificateChain() const {
	return m_socket->sslConfiguration().peerCertificateChain();
}

QSslCipher WebSocketConnection::sessionCipher() const {
	return m_socket->sslConfiguration().sessionCipher();
}

QSsl::SslProtocol WebSocketConnection::sessionProtocol() const {
	return m_socket->sslConfiguration().sessionProtocol();
}

QString WebSocketConnection::sessionProtocolString() const {
	return MumbleSSL::protocolToString(sessionProtocol());
}

QSslKey WebSocketConnection::ephemeralServerKey() const {
	return m_socket->sslConfiguration().ephemeralServerKey();
}
